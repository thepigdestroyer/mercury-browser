// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Reading another browser's profile, on a background thread: `mercury-import` copies its
//! databases into the profile's private copy area (see `copies`), reads the copies and removes
//! them. Logs name the profile ("Firefox — Default") and the kind of error, never a path into
//! the other browser's profile, which holds the user's name.

use mercury_import::{
    ImportError, MAX_IMPORTED_VISITS, ReadContext, read_chromium_bookmarks, read_chromium_history,
    read_firefox_bookmarks, read_firefox_history,
};
use mercury_store::BookmarkImport;

use super::convert::{Converted, bookmarks_for_store, visits_for_store};
use super::sources::{Family, FoundProfile};
use super::summary::describe_error;
use crate::profile_data::import::ImportVisit;

/// What to import from a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Choice {
    pub(crate) bookmarks: bool,
    pub(crate) history: bool,
}

/// What was read, ready for the profile database, or why it could not be read. `None` for a
/// kind that was not chosen.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) bookmarks: Option<Result<Converted<Vec<BookmarkImport>>, String>>,
    pub(crate) history: Option<Result<Converted<Vec<ImportVisit>>, String>>,
}

/// The kind of a read error, for the log.
fn error_kind(error: &ImportError) -> &'static str {
    match error {
        ImportError::NotFound(_) => "not found",
        ImportError::Locked(_) => "in use",
        ImportError::TooLarge { .. } => "too large",
        ImportError::Io { .. } => "input/output error",
        ImportError::Database { .. } => "damaged database",
        ImportError::Malformed(_) => "unexpected format",
        ImportError::NotARegularFile(_) => "not a regular file",
        ImportError::Cancelled => "cancelled",
    }
}

/// Reads what `choice` asks for from `profile`, copying databases under `context`. Visits later
/// than a day after `now_ms` are left out. Blocking: never call it on the UI thread.
pub(crate) fn read_profile(
    profile: &FoundProfile,
    choice: Choice,
    context: &ReadContext,
    now_ms: i64,
) -> Prepared {
    let browser = profile.browser.as_str();
    let failed = |what: &str, error: ImportError| {
        log::info!(
            "could not read the {what} of {}: {}",
            profile.label(),
            error_kind(&error)
        );
        describe_error(browser, &error)
    };
    let bookmarks = choice.bookmarks.then(|| {
        let read = match profile.family {
            Family::Firefox => read_firefox_bookmarks(&profile.path, context),
            Family::Chromium => read_chromium_bookmarks(&profile.path, context),
        };
        let nodes = read.map_err(|error| failed("bookmarks", error))?;
        bookmarks_for_store(nodes, &mut crate::profile_data::new_guid).map_err(|_| {
            "the system's random source is unavailable, so no bookmark could be added.".to_owned()
        })
    });
    let history = choice.history.then(|| {
        let read = match profile.family {
            Family::Firefox => read_firefox_history(&profile.path, MAX_IMPORTED_VISITS, context),
            Family::Chromium => read_chromium_history(&profile.path, MAX_IMPORTED_VISITS, context),
        };
        read.map(|visits| visits_for_store(visits, now_ms))
            .map_err(|error| failed("history", error))
    });
    Prepared { bookmarks, history }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_ui::fixtures::{self, discover};

    fn read_profile(profile: &FoundProfile, choice: Choice) -> Prepared {
        fixtures::read(profile, choice)
    }

    #[test]
    fn a_firefox_fixture_is_read_and_converted() {
        let root = tempfile::tempdir().unwrap();
        fixtures::firefox_profile(root.path(), "Default", fixtures::SAMPLE_FIREFOX);
        let profile = discover(&[root.path().to_owned()]).remove(0);

        let prepared = read_profile(
            &profile,
            Choice {
                bookmarks: true,
                history: true,
            },
        );

        let bookmarks = prepared.bookmarks.unwrap().unwrap();
        assert_eq!(
            crate::import_ui::convert::bookmark_count(&bookmarks.items),
            3
        );
        let visits = prepared.history.unwrap().unwrap();
        assert_eq!(visits.items.len(), 4);
        assert!(
            visits
                .items
                .windows(2)
                .all(|pair| pair[0].visited_at_ms >= pair[1].visited_at_ms),
            "newest first"
        );
    }

    #[test]
    fn only_the_chosen_kinds_are_read() {
        let root = tempfile::tempdir().unwrap();
        fixtures::chromium_profile(root.path(), "Default", "Person 1", true, true);
        let profile = discover(&[root.path().to_owned()]).remove(0);

        let prepared = read_profile(
            &profile,
            Choice {
                bookmarks: false,
                history: true,
            },
        );

        assert!(prepared.bookmarks.is_none());
        assert_eq!(prepared.history.unwrap().unwrap().items.len(), 3);
    }

    #[test]
    fn a_damaged_database_or_bookmarks_file_is_reported_per_kind() {
        let root = tempfile::tempdir().unwrap();
        let path = fixtures::chromium_profile(root.path(), "Default", "Person 1", true, true);
        std::fs::write(
            path.join("History"),
            b"this is not a database at all, not even close",
        )
        .unwrap();
        std::fs::write(path.join("Bookmarks"), b"{\"roots\": ").unwrap();
        let profile = discover(&[root.path().to_owned()]).remove(0);

        let prepared = read_profile(
            &profile,
            Choice {
                bookmarks: true,
                history: true,
            },
        );

        assert_eq!(
            prepared.bookmarks.unwrap().unwrap_err(),
            "the files are not in a form this version of Chrome is known to write."
        );
        assert_eq!(
            prepared.history.unwrap().unwrap_err(),
            "the database is damaged."
        );
    }

    #[test]
    fn a_profile_with_an_unknown_schema_is_reported() {
        let root = tempfile::tempdir().unwrap();
        fixtures::firefox_profile(root.path(), "Default", "DROP TABLE moz_historyvisits;");
        let profile = discover(&[root.path().to_owned()]).remove(0);

        let prepared = read_profile(
            &profile,
            Choice {
                bookmarks: false,
                history: true,
            },
        );

        assert_eq!(
            prepared.history.unwrap().unwrap_err(),
            "the files are not in a form this version of Firefox is known to write."
        );
    }
}
