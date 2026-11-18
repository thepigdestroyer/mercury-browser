// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which other browsers' profiles can be imported from, and where to look for them.
//!
//! Discovery only looks in the directories it is given. The platform's directories are taken
//! from the real environment only when the user opens the import window in a normal run
//! ([`Search::Platform`]); an automated run (screenshot, background) looks nowhere unless a
//! diagnostic option names a directory of fixtures ([`Search::Fixture`]).

use std::path::{Path, PathBuf};

use mercury_import::{BrowserProfile, ReadContext, find_chromium_profiles, find_firefox_profiles};

/// Which reader a profile needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Firefox,
    Chromium,
}

/// A profile that can be imported from, with what it holds as far as that is cheap to know
/// (which files are there; not whether they hold anything).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoundProfile {
    pub(crate) browser: String,
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) family: Family,
    pub(crate) bookmarks: bool,
    pub(crate) history: bool,
}

impl FoundProfile {
    /// "Firefox — Default".
    pub(crate) fn label(&self) -> String {
        format!("{} — {}", self.browser, self.name)
    }

    /// What the profile holds, for its row in the window.
    pub(crate) fn holds(&self) -> &'static str {
        match (self.bookmarks, self.history) {
            (true, true) => "Bookmarks and history",
            (true, false) => "Bookmarks",
            (false, true) => "History",
            (false, false) => "Nothing to import",
        }
    }
}

/// Where discovery looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Search {
    /// The platform's directories, from the real environment.
    Platform,
    /// One directory standing in for the platform's (a diagnostic run).
    Fixture(PathBuf),
    /// Nowhere: nothing is found.
    Nowhere,
}

/// The search for this run: an automated run never looks at the real environment.
pub(crate) fn search_for(automated: bool, fixture: Option<&Path>) -> Search {
    match (fixture, automated) {
        (Some(root), _) => Search::Fixture(root.to_owned()),
        (None, true) => Search::Nowhere,
        (None, false) => Search::Platform,
    }
}

/// The directories `search` looks in. Only [`Search::Platform`] reads the environment.
pub(crate) fn directories(search: &Search) -> Vec<PathBuf> {
    match search {
        Search::Platform => platform_directories(),
        Search::Fixture(root) => vec![root.clone()],
        Search::Nowhere => Vec::new(),
    }
}

/// The directories `mercury-import` documents: the application-support directory on macOS,
/// the config and home directories on Linux, `%LOCALAPPDATA%` and `%APPDATA%` on Windows.
fn platform_directories() -> Vec<PathBuf> {
    let directories = if cfg!(target_os = "macos") {
        vec![dirs::data_dir()]
    } else if cfg!(windows) {
        vec![dirs::data_local_dir(), dirs::data_dir()]
    } else {
        vec![dirs::config_dir(), dirs::home_dir()]
    };
    directories.into_iter().flatten().collect()
}

/// The profiles under `base_dirs`, Firefox-family browsers first. With no directories nothing
/// is read and nothing is found. Firefox's profile-group databases are copied into
/// `context`'s directory to be read.
pub(crate) fn discover(base_dirs: &[PathBuf], context: &ReadContext) -> Vec<FoundProfile> {
    if base_dirs.is_empty() {
        return Vec::new();
    }
    let firefox = find_firefox_profiles(base_dirs, context)
        .into_iter()
        .map(|profile| found(profile, Family::Firefox));
    let chromium = find_chromium_profiles(base_dirs)
        .into_iter()
        .map(|profile| found(profile, Family::Chromium));
    firefox.chain(chromium).collect()
}

fn found(profile: BrowserProfile, family: Family) -> FoundProfile {
    let has = |file: &str| profile.path.join(file).is_file();
    let (bookmarks, history) = match family {
        Family::Firefox => (has("places.sqlite"), has("places.sqlite")),
        // Arc keeps its bookmarks in a sidebar file the reader does not read.
        Family::Chromium => (
            profile.browser != "Arc" && (has("Bookmarks") || has("AccountBookmarks")),
            has("History"),
        ),
    };
    FoundProfile {
        browser: profile.browser,
        name: profile.name,
        path: profile.path,
        family,
        bookmarks,
        history,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_ui::fixtures::{self, discover};

    #[test]
    fn firefox_and_chromium_layouts_are_found_with_what_they_hold() {
        let root = tempfile::tempdir().unwrap();
        fixtures::firefox_profile(root.path(), "Default User", fixtures::SAMPLE_FIREFOX);
        fixtures::chromium_profile(root.path(), "Default", "Work", true, true);
        fixtures::chromium_profile(root.path(), "Profile 1", "Reading", true, false);

        let found = discover(&[root.path().to_owned()]);

        let rows = found
            .iter()
            .map(|profile| (profile.label(), profile.family, profile.holds()))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                (
                    "Firefox — Default User".to_owned(),
                    Family::Firefox,
                    "Bookmarks and history"
                ),
                (
                    "Chrome — Work".to_owned(),
                    Family::Chromium,
                    "Bookmarks and history"
                ),
                ("Chrome — Reading".to_owned(), Family::Chromium, "Bookmarks"),
            ]
        );
        assert!(
            found
                .iter()
                .all(|profile| profile.path.starts_with(root.path()))
        );
    }

    #[test]
    fn an_empty_directory_has_no_profiles() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(discover(&[root.path().to_owned()]), Vec::new());
    }

    #[test]
    fn with_no_directories_nothing_is_looked_at() {
        assert_eq!(discover(&[]), Vec::new());
        // Nothing is read, so even a context whose copy directory does not exist is never used.
        let nowhere = ReadContext::new(PathBuf::from("/nonexistent/copies"));
        assert_eq!(super::discover(&[], &nowhere), Vec::new());
        assert_eq!(directories(&Search::Nowhere), Vec::<PathBuf>::new());
    }

    #[test]
    fn automated_runs_never_search_the_real_environment() {
        assert_eq!(search_for(true, None), Search::Nowhere);
        let fixture = Path::new("/fixture/root");
        assert_eq!(
            search_for(true, Some(fixture)),
            Search::Fixture(fixture.to_owned())
        );
        assert_eq!(
            directories(&search_for(true, Some(fixture))),
            vec![fixture.to_owned()]
        );
        assert_eq!(search_for(false, None), Search::Platform);
    }
}
