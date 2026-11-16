//! Fixture profiles of other browsers, built in a test's own temporary directory (never a real
//! profile). The tables follow Firefox's `nsPlacesTables.h` and Chromium's `History` schema,
//! as `mercury-import`'s own tests build them.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

const FIREFOX_SCHEMA: &str = "
    CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR,
        rev_host LONGVARCHAR, visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL,
        typed INTEGER DEFAULT 0 NOT NULL, frecency INTEGER DEFAULT -1 NOT NULL,
        last_visit_date INTEGER, guid TEXT, foreign_count INTEGER DEFAULT 0 NOT NULL,
        url_hash INTEGER DEFAULT 0 NOT NULL);
    CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, from_visit INTEGER,
        place_id INTEGER, visit_date INTEGER, visit_type INTEGER, session INTEGER,
        source INTEGER DEFAULT 0 NOT NULL);
    CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER DEFAULT NULL,
        parent INTEGER, position INTEGER, title LONGVARCHAR, keyword_id INTEGER,
        folder_type TEXT, dateAdded INTEGER, lastModified INTEGER, guid TEXT,
        syncStatus INTEGER NOT NULL DEFAULT 0, syncChangeCounter INTEGER NOT NULL DEFAULT 1);
    INSERT INTO moz_bookmarks (id, type, parent, position, title, dateAdded, guid) VALUES
        (1, 2, 0, 0, '', 1690000000000000, 'root________'),
        (2, 2, 1, 0, 'menu', 1690000000000000, 'menu________'),
        (3, 2, 1, 1, 'toolbar', 1690000000000000, 'toolbar_____'),
        (4, 2, 1, 2, 'tags', 1690000000000000, 'tags________'),
        (5, 2, 1, 3, 'unfiled', 1690000000000000, 'unfiled_____'),
        (6, 2, 1, 4, 'mobile', 1690000000000000, 'mobile______');
";

/// Three bookmarks (two on the toolbar, one in a folder there, and a separator) and four
/// visits to three pages.
pub(crate) const SAMPLE_FIREFOX: &str = "
    INSERT INTO moz_places (id, url, title) VALUES
        (1, 'https://news.example/', 'News'),
        (2, 'https://docs.example/', 'Docs'),
        (3, 'https://mail.example/', 'Mail');
    INSERT INTO moz_bookmarks (id, type, fk, parent, position, title, dateAdded, guid) VALUES
        (10, 1, 1, 3, 0, 'News', 1700000000000000, 'news________'),
        (11, 3, NULL, 3, 1, NULL, 1700000000000000, 'separator___'),
        (12, 2, NULL, 3, 2, 'Reading', 1700000000000000, 'reading_____'),
        (13, 1, 2, 12, 0, 'Docs', 1700000000000000, 'docs________'),
        (20, 1, 3, 2, 0, 'Mail', 1700000000000000, 'mail________');
    INSERT INTO moz_historyvisits (place_id, visit_date, visit_type) VALUES
        (1, 1700000000000000, 1),
        (1, 1700000500000000, 1),
        (2, 1700000300000000, 2),
        (3, 1700000400000000, 1);
";

/// A Firefox install under `root` (as the application-support directory) with one default
/// profile named `name`, whose `places.sqlite` is filled by `sql`. Returns the profile path.
pub(crate) fn firefox_profile(root: &Path, name: &str, sql: &str) -> PathBuf {
    let tree = root.join("Firefox");
    let profile = tree.join("Profiles").join("abcd1234.default-release");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(
        tree.join("profiles.ini"),
        format!(
            "[Profile0]\nName={name}\nIsRelative=1\nPath=Profiles/abcd1234.default-release\n\
             Default=1\n\n[General]\nStartWithLastProfile=1\nVersion=2\n"
        ),
    )
    .unwrap();
    let conn = Connection::open(profile.join("places.sqlite")).unwrap();
    conn.execute_batch(FIREFOX_SCHEMA).unwrap();
    conn.execute_batch(sql).unwrap();
    profile
}

/// A Chrome profile directory `dir` under `root`, shown as `name`, with a `Bookmarks` file
/// (two bookmarks on the bar, one in "Other bookmarks") and a `History` database (three
/// visits) as asked. Returns the profile path.
pub(crate) fn chromium_profile(
    root: &Path,
    dir: &str,
    name: &str,
    bookmarks: bool,
    history: bool,
) -> PathBuf {
    let user_data = root.join("Google").join("Chrome");
    let profile = user_data.join(dir);
    std::fs::create_dir_all(&profile).unwrap();
    let state_path = user_data.join("Local State");
    let mut state: serde_json::Value = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({ "profile": { "info_cache": {} } }));
    state["profile"]["info_cache"][dir] = serde_json::json!({ "name": name });
    std::fs::write(&state_path, state.to_string()).unwrap();
    if bookmarks {
        let file = serde_json::json!({
            "roots": {
                "bookmark_bar": { "name": "Bookmarks bar", "type": "folder", "children": [
                    { "type": "url", "name": "Search", "url": "https://search.example/",
                      "date_added": "13344473600000000" },
                    { "type": "url", "name": "Maps", "url": "https://maps.example/",
                      "date_added": "13344473600000000" }
                ]},
                "other": { "name": "Other bookmarks", "type": "folder", "children": [
                    { "type": "url", "name": "Recipes", "url": "https://recipes.example/",
                      "date_added": "13344473600000000" }
                ]},
                "synced": { "name": "Mobile bookmarks", "type": "folder", "children": [] }
            },
            "version": 1
        });
        std::fs::write(profile.join("Bookmarks"), file.to_string()).unwrap();
    }
    if history {
        let conn = Connection::open(profile.join("History")).unwrap();
        // Times are microseconds since 1601: 13344473600000000 is 2023-11-17.
        conn.execute_batch(
            "CREATE TABLE urls(id INTEGER PRIMARY KEY AUTOINCREMENT, url LONGVARCHAR,
                 title LONGVARCHAR, visit_count INTEGER DEFAULT 0 NOT NULL,
                 typed_count INTEGER DEFAULT 0 NOT NULL, last_visit_time INTEGER NOT NULL,
                 hidden INTEGER DEFAULT 0 NOT NULL);
             CREATE TABLE visits(id INTEGER PRIMARY KEY AUTOINCREMENT, url INTEGER NOT NULL,
                 visit_time INTEGER NOT NULL, from_visit INTEGER,
                 transition INTEGER DEFAULT 0 NOT NULL, segment_id INTEGER,
                 visit_duration INTEGER DEFAULT 0 NOT NULL);
             INSERT INTO urls (id, url, title, last_visit_time) VALUES
                 (1, 'https://search.example/', 'Search', 0),
                 (2, 'https://maps.example/', 'Maps', 0);
             INSERT INTO visits (url, visit_time) VALUES
                 (1, 13344473600000000), (1, 13344473700000000), (2, 13344473800000000);",
        )
        .unwrap();
    }
    profile
}

/// Discovery under `dirs`, with copies in a temporary directory of the test's own.
pub(crate) fn discover(dirs: &[PathBuf]) -> Vec<super::sources::FoundProfile> {
    let copies = tempfile::tempdir().unwrap();
    super::sources::discover(
        dirs,
        &mercury_import::ReadContext::new(copies.path().to_owned()),
    )
}

/// Reads `profile` as the import does, with copies in a temporary directory of the test's
/// own, which must be empty afterwards.
pub(crate) fn read(
    profile: &super::sources::FoundProfile,
    choice: super::read::Choice,
) -> super::read::Prepared {
    let copies = tempfile::tempdir().unwrap();
    let context = mercury_import::ReadContext::new(copies.path().to_owned());
    let prepared =
        super::read::read_profile(profile, choice, &context, crate::profile_data::now_ms());
    assert_eq!(
        std::fs::read_dir(copies.path()).unwrap().count(),
        0,
        "the copies are removed after a read"
    );
    prepared
}
