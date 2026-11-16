//! A note in the profile directory of the imports that have not finished, one entry per source
//! profile, so that the next time the import window opens it can offer to continue them. An
//! entry is written when an import starts and removed when that profile's import ends without
//! being interrupted; an import cancelled at quit leaves it. Continuing runs the import again:
//! what the earlier run committed is recognised and left out (`profile_data::import_merge`), so
//! it resumes where it stopped.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FILE_NAME: &str = "unfinished-import.json";
const VERSION: u64 = 2;

/// An import that did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unfinished {
    pub(crate) browser: String,
    pub(crate) name: String,
    /// The other browser's profile directory, which identifies the entry.
    pub(crate) path: PathBuf,
    pub(crate) bookmarks: bool,
    pub(crate) history: bool,
}

/// Every readable entry; a damaged note reads as none.
pub(crate) fn load(profile_dir: &Path) -> Vec<Unfinished> {
    let Some(value) = std::fs::read(profile_dir.join(FILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return Vec::new();
    };
    if value.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Vec::new();
    }
    let Some(entries) = value.get("imports").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let text = |key: &str| entry.get(key)?.as_str().map(str::to_owned);
            let flag = |key: &str| entry.get(key)?.as_bool();
            Some(Unfinished {
                browser: text("browser")?,
                name: text("name")?,
                path: PathBuf::from(text("path")?),
                bookmarks: flag("bookmarks")?,
                history: flag("history")?,
            })
        })
        .collect()
}

fn store(profile_dir: &Path, entries: &[Unfinished]) -> std::io::Result<()> {
    let path = profile_dir.join(FILE_NAME);
    if entries.is_empty() {
        return match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    let imports = entries
        .iter()
        // Paths that are not UTF-8 cannot be written to JSON; such an import can still be run
        // again by hand.
        .filter_map(|entry| {
            Some(json!({
                "browser": entry.browser,
                "name": entry.name,
                "path": entry.path.to_str()?,
                "bookmarks": entry.bookmarks,
                "history": entry.history,
            }))
        })
        .collect::<Vec<_>>();
    let text = json!({ "version": VERSION, "imports": imports }).to_string();
    let temporary = profile_dir.join(format!("{FILE_NAME}.new"));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// Notes `unfinished`, replacing an earlier entry for the same profile.
pub(crate) fn add(profile_dir: &Path, unfinished: &Unfinished) -> std::io::Result<()> {
    let mut entries = load(profile_dir);
    entries.retain(|entry| entry.path != unfinished.path);
    entries.push(unfinished.clone());
    store(profile_dir, &entries)
}

/// Removes the entry for the profile at `path`, if any.
pub(crate) fn remove(profile_dir: &Path, path: &Path) -> std::io::Result<()> {
    let mut entries = load(profile_dir);
    let before = entries.len();
    entries.retain(|entry| entry.path != path);
    if entries.len() == before {
        return Ok(());
    }
    store(profile_dir, &entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(path: &str) -> Unfinished {
        Unfinished {
            browser: "Firefox".into(),
            name: "Default".into(),
            path: PathBuf::from(path),
            bookmarks: true,
            history: false,
        }
    }

    #[test]
    fn entries_are_kept_per_profile_and_removed_one_by_one() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Vec::new());

        add(dir.path(), &sample("/fixtures/a")).unwrap();
        add(dir.path(), &sample("/fixtures/b")).unwrap();
        let again = Unfinished {
            history: true,
            ..sample("/fixtures/a")
        };
        add(dir.path(), &again).unwrap();
        assert_eq!(load(dir.path()), vec![sample("/fixtures/b"), again]);

        remove(dir.path(), Path::new("/fixtures/b")).unwrap();
        assert_eq!(load(dir.path()).len(), 1);
        remove(dir.path(), Path::new("/fixtures/a")).unwrap();
        assert_eq!(load(dir.path()), Vec::new());
        assert!(
            !dir.path().join(FILE_NAME).exists(),
            "no empty note is left"
        );
        remove(dir.path(), Path::new("/fixtures/a")).unwrap();
    }

    #[test]
    fn a_damaged_or_older_note_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), b"{\"version\": 2, \"imports\"").unwrap();
        assert_eq!(load(dir.path()), Vec::new());
        std::fs::write(
            dir.path().join(FILE_NAME),
            b"{\"version\": 1, \"browser\": \"x\"}",
        )
        .unwrap();
        assert_eq!(load(dir.path()), Vec::new());
    }
}
