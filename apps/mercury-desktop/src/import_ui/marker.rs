//! A note in the profile directory of an import that has not finished, so that the next time
//! the import window opens it can offer to continue. It is written when an import starts and
//! removed when one ends without being interrupted; an import cancelled at quit leaves it.
//! Continuing runs the import again: what the earlier run committed is recognised and left out
//! (`profile_data::import_merge`), so it resumes where it stopped.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FILE_NAME: &str = "unfinished-import.json";

/// The import that did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unfinished {
    pub(crate) browser: String,
    pub(crate) name: String,
    /// The other browser's profile directory.
    pub(crate) path: PathBuf,
    pub(crate) bookmarks: bool,
    pub(crate) history: bool,
}

pub(crate) fn save(profile_dir: &Path, unfinished: &Unfinished) -> std::io::Result<()> {
    let Some(path) = unfinished.path.to_str() else {
        // Paths that are not UTF-8 cannot be written to JSON; the import can still be run again.
        return Ok(());
    };
    let text = json!({
        "version": 1,
        "browser": unfinished.browser,
        "name": unfinished.name,
        "path": path,
        "bookmarks": unfinished.bookmarks,
        "history": unfinished.history,
    })
    .to_string();
    let temporary = profile_dir.join(format!("{FILE_NAME}.new"));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, profile_dir.join(FILE_NAME))
}

/// The note, if there is a readable one.
pub(crate) fn load(profile_dir: &Path) -> Option<Unfinished> {
    let bytes = std::fs::read(profile_dir.join(FILE_NAME)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    if value.get("version")?.as_u64()? != 1 {
        return None;
    }
    let text = |key: &str| value.get(key)?.as_str().map(str::to_owned);
    let flag = |key: &str| value.get(key)?.as_bool();
    Some(Unfinished {
        browser: text("browser")?,
        name: text("name")?,
        path: PathBuf::from(text("path")?),
        bookmarks: flag("bookmarks")?,
        history: flag("history")?,
    })
}

pub(crate) fn clear(profile_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(profile_dir.join(FILE_NAME)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Unfinished {
        Unfinished {
            browser: "Firefox".into(),
            name: "Default".into(),
            path: PathBuf::from("/fixtures/Firefox/Profiles/a.default"),
            bookmarks: true,
            history: false,
        }
    }

    #[test]
    fn a_saved_note_is_loaded_back_and_cleared() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);

        save(dir.path(), &sample()).unwrap();
        assert_eq!(load(dir.path()), Some(sample()));

        clear(dir.path()).unwrap();
        assert_eq!(load(dir.path()), None);
        clear(dir.path()).unwrap();
    }

    #[test]
    fn a_damaged_note_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), b"{\"version\": 1, \"browser\"").unwrap();
        assert_eq!(load(dir.path()), None);
        std::fs::write(dir.path().join(FILE_NAME), b"{\"version\": 2}").unwrap();
        assert_eq!(load(dir.path()), None);
    }
}
