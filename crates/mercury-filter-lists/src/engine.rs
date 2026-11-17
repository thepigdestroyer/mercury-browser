//! Choosing the copy of each list to use, and the compiled engine built from them.

use std::path::Path;

use adblock::Engine;
use adblock::lists::FilterSet;

use crate::cache::{Cache, CacheState};
use crate::list_text::network_options;
use crate::manifest::{ListEntry, MANIFEST_FILE, MAX_MANIFEST_BYTES, Manifest, parse_manifest};
use crate::sha256_hex;

/// Part of every engine key: a compiled engine is only read back by the adblock version and
/// rule selection that wrote it.
const ENGINE_FORMAT: &str = "adblock-rust =0.13.5, network rules only, format 1";

/// Where the copy in use came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The file that ships with the browser.
    Bundled,
    /// An update in the profile's cache.
    Updated,
}

/// The copy of one list in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListInUse {
    pub entry: ListEntry,
    pub text: String,
    pub sha256: String,
    pub origin: Origin,
    /// The list's version header, if any.
    pub version: Option<String>,
    /// When this copy was taken (bundled) or last confirmed current (updated).
    pub date: u64,
}

/// Why no engine could be built.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("the filter-list manifest {0} could not be read: {1}")]
    ManifestUnreadable(String, String),
    #[error("the filter-list manifest is not usable: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
    #[error("none of the filter lists could be read")]
    NoLists,
}

/// Reads and checks the bundled manifest.
pub fn read_manifest(bundled_dir: &Path) -> Result<Manifest, LoadError> {
    let path = bundled_dir.join(MANIFEST_FILE);
    let unreadable = |reason: String| LoadError::ManifestUnreadable(path.display().to_string(), reason);
    let metadata = std::fs::metadata(&path).map_err(|error| unreadable(error.to_string()))?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(unreadable("it is too large".to_owned()));
    }
    let text = std::fs::read_to_string(&path).map_err(|error| unreadable(error.to_string()))?;
    Ok(parse_manifest(&text)?)
}

/// The newest valid copy of each list: the updated copy in the cache if it is intact, was
/// fetched for the manifest's source URL, and is newer than the bundled snapshot; otherwise
/// the bundled file if its SHA-256 matches the manifest. A list with neither is left out (and
/// logged).
pub fn choose_lists(
    manifest: &Manifest,
    bundled_dir: &Path,
    cache: Option<(&Cache, &CacheState)>,
) -> Vec<ListInUse> {
    manifest
        .lists
        .iter()
        .filter_map(|entry| {
            let updated = cache.and_then(|(cache, state)| updated_copy(entry, cache, state));
            let chosen = updated.or_else(|| bundled_copy(entry, bundled_dir));
            if chosen.is_none() {
                log::error!(
                    "filter lists: {} is left out: its bundled file is missing or damaged",
                    entry.name
                );
            }
            chosen
        })
        .collect()
}

fn updated_copy(entry: &ListEntry, cache: &Cache, state: &CacheState) -> Option<ListInUse> {
    let cached = state.lists.get(&entry.id)?;
    if cached.source_url != entry.source_url.as_str() || cached.current_as_of <= entry.date {
        return None;
    }
    let text = cache.read_list(cached)?;
    Some(ListInUse {
        entry: entry.clone(),
        text,
        sha256: cached.sha256.clone(),
        origin: Origin::Updated,
        version: cached.version.clone(),
        date: cached.current_as_of,
    })
}

fn bundled_copy(entry: &ListEntry, bundled_dir: &Path) -> Option<ListInUse> {
    let bytes = std::fs::read(bundled_dir.join(&entry.file)).ok()?;
    if sha256_hex(&bytes) != entry.sha256 {
        return None;
    }
    Some(ListInUse {
        entry: entry.clone(),
        text: String::from_utf8(bytes).ok()?,
        sha256: entry.sha256.clone(),
        origin: Origin::Bundled,
        version: entry.version.clone(),
        date: entry.date,
    })
}

/// Identifies a set of copies: equal keys mean the same compiled engine.
pub fn engine_key(lists: &[ListInUse]) -> String {
    let mut description = format!("{ENGINE_FORMAT}\n");
    for list in lists {
        description.push_str(&format!("{} {}\n", list.entry.id, list.sha256));
    }
    sha256_hex(description.as_bytes())
}

/// Compiles the network rules of `lists` into a serialized engine.
pub fn compile(lists: &[ListInUse]) -> Vec<u8> {
    let mut filter_set = FilterSet::new(false);
    for list in lists {
        let text = list.text.trim_start_matches('\u{feff}').to_owned();
        filter_set.add_filter_list(text, network_options());
    }
    Engine::new_with_filter_set(filter_set).serialize()
}

/// Whether `bytes` is an engine this adblock version can load.
pub fn is_loadable(bytes: &[u8]) -> bool {
    // adblock-rust checks the data while deserializing; a panic on damaged data is treated as
    // "not loadable" rather than taking the updater down.
    std::panic::catch_unwind(|| Engine::default().deserialize(bytes).is_ok()).unwrap_or(false)
}

/// The compiled engine for `lists`: read back from the cache when it holds an intact, loadable
/// engine for the same key, otherwise compiled and stored there (the state's engine entry is
/// updated; the caller saves the state).
pub fn engine_for(lists: &[ListInUse], cache: Option<(&Cache, &mut CacheState)>) -> Vec<u8> {
    let key = engine_key(lists);
    let Some((cache, state)) = cache else {
        return compile(lists);
    };
    if let Some(bytes) = cache.read_engine(state, &key) {
        if is_loadable(&bytes) {
            log::info!("filter lists: using the compiled engine from the cache");
            return bytes;
        }
        log::warn!("filter lists: the cached engine does not load; compiling it again");
    }
    let bytes = compile(lists);
    log::info!("filter lists: compiled the engine from the lists");
    match cache.write_engine(&key, &bytes) {
        Ok(entry) => state.engine = Some(entry),
        Err(error) => log::warn!("filter lists: could not cache the compiled engine: {error}"),
    }
    bytes
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cache::CachedList;

    pub(crate) const SNAPSHOT: u64 = 1_794_775_611;

    /// A bundled directory with lists `(id, text)`, all dated `SNAPSHOT`.
    pub(crate) fn bundle(directory: &Path, lists: &[(&str, &str)]) -> Manifest {
        std::fs::create_dir_all(directory).unwrap();
        let entries = lists
            .iter()
            .map(|(id, text)| {
                std::fs::write(directory.join(format!("{id}.txt")), text).unwrap();
                format!(
                    r#"{{"id": "{id}", "name": "List {id}", "file": "{id}.txt",
                        "source_url": "https://lists.example/{id}.txt", "licence": "CC0-1.0",
                        "version": null, "date": "{}", "sha256": "{}"}}"#,
                    crate::timestamp::format_utc(SNAPSHOT),
                    sha256_hex(text.as_bytes())
                )
            })
            .collect::<Vec<_>>();
        let text = format!(r#"{{"format_version": 1, "lists": [{}]}}"#, entries.join(","));
        std::fs::write(directory.join(MANIFEST_FILE), text).unwrap();
        read_manifest(directory).unwrap()
    }

    pub(crate) fn blocks(engine: &[u8], url: &str) -> bool {
        let mut loaded = Engine::default();
        loaded.deserialize(engine).expect("a loadable engine");
        let request =
            adblock::request::Request::new(url, "https://page.example/", "script", "GET").unwrap();
        loaded.check_network_request(&request).should_block()
    }

    fn updated(cache: &Cache, id: &str, text: &str, current_as_of: u64) -> CachedList {
        let (file, sha256) = cache.write_list(id, text).unwrap();
        CachedList {
            file,
            sha256,
            source_url: format!("https://lists.example/{id}.txt"),
            etag: None,
            last_modified: None,
            version: Some("v2".to_owned()),
            current_as_of,
        }
    }

    fn setup() -> (tempfile::TempDir, Manifest, Cache) {
        let directory = tempfile::tempdir().unwrap();
        let manifest = bundle(
            &directory.path().join("bundled"),
            &[("ads", "||ads.example^\n"), ("trackers", "||trackers.example^\n")],
        );
        let cache = Cache::open(&directory.path().join("cache")).unwrap();
        (directory, manifest, cache)
    }

    #[test]
    fn without_a_cache_the_bundled_lists_are_used_in_manifest_order() {
        let (directory, manifest, _cache) = setup();

        let lists = choose_lists(&manifest, &directory.path().join("bundled"), None);

        assert_eq!(
            lists.iter().map(|list| (list.entry.id.as_str(), list.origin)).collect::<Vec<_>>(),
            [("ads", Origin::Bundled), ("trackers", Origin::Bundled)]
        );
        assert_eq!(lists[0].text, "||ads.example^\n");
        assert_eq!(lists[0].date, SNAPSHOT);
    }

    #[test]
    fn a_newer_intact_update_is_used_instead_of_the_bundled_copy() {
        let (directory, manifest, cache) = setup();
        let mut state = CacheState::default();
        state.lists.insert(
            "ads".to_owned(),
            updated(&cache, "ads", "||ads.example^\n||more-ads.example^\n", SNAPSHOT + 10),
        );

        let lists = choose_lists(&manifest, &directory.path().join("bundled"), Some((&cache, &state)));

        assert_eq!(lists[0].origin, Origin::Updated);
        assert_eq!(lists[0].version.as_deref(), Some("v2"));
        assert_eq!(lists[0].date, SNAPSHOT + 10);
        assert!(lists[0].text.contains("more-ads"));
        assert_eq!(lists[1].origin, Origin::Bundled);
    }

    #[test]
    fn an_update_older_than_the_bundled_snapshot_is_not_used() {
        let (directory, manifest, cache) = setup();
        let mut state = CacheState::default();
        state
            .lists
            .insert("ads".to_owned(), updated(&cache, "ads", "||old.example^\n", SNAPSHOT - 10));

        let lists = choose_lists(&manifest, &directory.path().join("bundled"), Some((&cache, &state)));

        assert_eq!(lists[0].origin, Origin::Bundled);
    }

    #[test]
    fn an_update_for_another_source_url_is_not_used() {
        let (directory, manifest, cache) = setup();
        let mut state = CacheState::default();
        let mut list = updated(&cache, "ads", "||ads.example^\n", SNAPSHOT + 10);
        list.source_url = "https://elsewhere.example/ads.txt".to_owned();
        state.lists.insert("ads".to_owned(), list);

        let lists = choose_lists(&manifest, &directory.path().join("bundled"), Some((&cache, &state)));

        assert_eq!(lists[0].origin, Origin::Bundled);
    }

    #[test]
    fn a_damaged_update_falls_back_to_the_bundled_copy() {
        let (directory, manifest, cache) = setup();
        let mut state = CacheState::default();
        let list = updated(&cache, "ads", "||ads.example^\n||new.example^\n", SNAPSHOT + 10);
        std::fs::write(cache.directory().join(&list.file), "||ads.exa").unwrap();
        state.lists.insert("ads".to_owned(), list);

        let lists = choose_lists(&manifest, &directory.path().join("bundled"), Some((&cache, &state)));

        assert_eq!(lists[0].origin, Origin::Bundled);
        assert_eq!(lists[0].text, "||ads.example^\n");
    }

    #[test]
    fn a_damaged_bundled_list_without_an_update_is_left_out() {
        let (directory, manifest, _cache) = setup();
        std::fs::write(directory.path().join("bundled/ads.txt"), "tampered").unwrap();

        let lists = choose_lists(&manifest, &directory.path().join("bundled"), None);

        assert_eq!(
            lists.iter().map(|list| list.entry.id.as_str()).collect::<Vec<_>>(),
            ["trackers"]
        );
    }

    #[test]
    fn a_missing_or_damaged_manifest_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_manifest(directory.path()),
            Err(LoadError::ManifestUnreadable(..))
        ));
        std::fs::write(directory.path().join(MANIFEST_FILE), "{").unwrap();
        assert!(matches!(read_manifest(directory.path()), Err(LoadError::Manifest(_))));
    }

    #[test]
    fn the_compiled_engine_blocks_the_lists_network_rules() {
        let (directory, manifest, _cache) = setup();
        let lists = choose_lists(&manifest, &directory.path().join("bundled"), None);

        let engine = compile(&lists);

        assert!(is_loadable(&engine));
        assert!(blocks(&engine, "https://ads.example/a.js"));
        assert!(blocks(&engine, "https://trackers.example/t.js"));
        assert!(!blocks(&engine, "https://page.example/app.js"));
        assert!(!is_loadable(b"not an engine"));
        assert!(!is_loadable(&engine[..engine.len() / 2]));
    }

    #[test]
    fn the_key_changes_with_any_list_and_only_then() {
        let (directory, manifest, cache) = setup();
        let bundled = choose_lists(&manifest, &directory.path().join("bundled"), None);
        let again = choose_lists(&manifest, &directory.path().join("bundled"), None);
        let mut state = CacheState::default();
        state
            .lists
            .insert("ads".to_owned(), updated(&cache, "ads", "||x.example^\n", SNAPSHOT + 1));
        let changed = choose_lists(&manifest, &directory.path().join("bundled"), Some((&cache, &state)));

        assert_eq!(engine_key(&bundled), engine_key(&again));
        assert_ne!(engine_key(&bundled), engine_key(&changed));
        assert_ne!(engine_key(&bundled), engine_key(&bundled[..1]));
    }

    #[test]
    fn the_engine_is_compiled_once_and_then_read_from_the_cache() {
        let (directory, manifest, cache) = setup();
        let lists = choose_lists(&manifest, &directory.path().join("bundled"), None);
        let mut state = CacheState::default();

        let first = engine_for(&lists, Some((&cache, &mut state)));
        let entry = state.engine.clone().expect("stored in the cache");
        // Replace the cached engine by a valid but different one: if it is read back, the
        // result shows it.
        let other = compile(&lists[1..]);
        let replaced = cache.write_engine(&entry.key, &other).unwrap();
        state.engine = Some(replaced);

        let second = engine_for(&lists, Some((&cache, &mut state)));

        assert!(blocks(&first, "https://ads.example/a.js"));
        assert_eq!(second, other, "read back from the cache, not compiled again");
    }

    #[test]
    fn a_damaged_cached_engine_is_compiled_again() {
        let (directory, manifest, cache) = setup();
        let lists = choose_lists(&manifest, &directory.path().join("bundled"), None);
        let mut state = CacheState::default();
        let first = engine_for(&lists, Some((&cache, &mut state)));
        let file = state.engine.as_ref().unwrap().file.clone();
        std::fs::write(cache.directory().join(file), b"garbage").unwrap();

        let second = engine_for(&lists, Some((&cache, &mut state)));

        assert!(is_loadable(&second));
        assert_eq!(second, first);
    }

    #[test]
    fn a_cached_engine_that_does_not_load_is_compiled_again() {
        let (directory, manifest, cache) = setup();
        let lists = choose_lists(&manifest, &directory.path().join("bundled"), None);
        let key = engine_key(&lists);
        let mut state = CacheState {
            engine: Some(cache.write_engine(&key, b"intact but not an engine").unwrap()),
            ..CacheState::default()
        };

        let engine = engine_for(&lists, Some((&cache, &mut state)));

        assert!(blocks(&engine, "https://ads.example/a.js"));
    }
}
