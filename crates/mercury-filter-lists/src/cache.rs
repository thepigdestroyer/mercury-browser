//! The profile's filter-list cache: updated copies of the lists, the compiled engine, and a
//! small state file that says which copy is which.
//!
//! Every file is replaced atomically (`write_private_atomic`). Copies are named after their
//! content (`<id>-<first 16 hex digits of the SHA-256>.txt`), and the state file is written
//! only after the copy it points to, so a crash at any moment leaves the state pointing at a
//! complete file. A copy whose SHA-256 does not match the state is damaged and is not used.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use mercury_settings::write_private_atomic;
use serde::{Deserialize, Serialize};

use crate::manifest::{is_safe_id, is_sha256_hex};
use crate::sha256_hex;

/// The state file's name in the cache directory.
pub const STATE_FILE: &str = "state.json";
const STATE_FORMAT_VERSION: u32 = 1;
/// The largest state file read, in bytes.
const MAX_STATE_BYTES: u64 = 256 * 1024;
/// The longest validator (`ETag`, `Last-Modified`) or message kept.
const MAX_FIELD_BYTES: usize = 512;

/// What the cache knows. Unknown or damaged state reads as empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheState {
    pub format_version: u32,
    /// When the last update check started, in seconds since the Unix epoch.
    pub last_check: Option<u64>,
    /// When the last check that reached every list ended.
    pub last_success: Option<u64>,
    /// Why the last check did not reach every list, for the settings window.
    pub last_error: Option<String>,
    /// Updated copies, by list id.
    pub lists: BTreeMap<String, CachedList>,
    /// The compiled engine of the copies in use.
    pub engine: Option<CachedEngine>,
}

impl Default for CacheState {
    fn default() -> Self {
        Self {
            format_version: STATE_FORMAT_VERSION,
            last_check: None,
            last_success: None,
            last_error: None,
            lists: BTreeMap::new(),
            engine: None,
        }
    }
}

/// One updated copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedList {
    /// The file in the cache directory.
    pub file: String,
    pub sha256: String,
    /// The URL it was fetched from; a copy from another URL than the manifest's is not used.
    pub source_url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// The list's version header.
    pub version: Option<String>,
    /// When the publisher last confirmed this copy as current (downloaded, or answered
    /// "not modified"), in seconds since the Unix epoch.
    pub current_as_of: u64,
}

/// The compiled engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedEngine {
    pub file: String,
    /// The key of the lists it was compiled from (see `compile::engine_key`).
    pub key: String,
    pub sha256: String,
}

/// The cache directory of one profile.
#[derive(Clone, Debug)]
pub struct Cache {
    directory: PathBuf,
}

impl Cache {
    /// Uses `directory`, creating it (private) if needed.
    pub fn open(directory: &Path) -> io::Result<Self> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        let metadata = std::fs::symlink_metadata(directory)?;
        if !metadata.is_dir() {
            return Err(io::Error::other("the filter-list cache is not a directory"));
        }
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The state, or an empty state if the file is missing, unreadable or damaged. Entries that
    /// name unsafe files or malformed hashes are dropped.
    pub fn load_state(&self) -> CacheState {
        let path = self.directory.join(STATE_FILE);
        let Some(bytes) = read_bounded(&path, MAX_STATE_BYTES) else {
            return CacheState::default();
        };
        let mut state = match serde_json::from_slice::<CacheState>(&bytes) {
            Ok(state) if state.format_version == STATE_FORMAT_VERSION => state,
            Ok(state) => {
                log::warn!(
                    "filter lists: the cache state has format {}; starting afresh",
                    state.format_version
                );
                return CacheState::default();
            }
            Err(error) => {
                log::warn!("filter lists: the cache state is damaged ({error}); starting afresh");
                return CacheState::default();
            }
        };
        state.lists.retain(|id, list| {
            is_safe_id(id)
                && is_sha256_hex(&list.sha256)
                && is_cache_file_name(&list.file, ".txt")
        });
        if let Some(engine) = &state.engine
            && !(is_sha256_hex(&engine.sha256) && is_cache_file_name(&engine.file, ".dat"))
        {
            state.engine = None;
        }
        state
    }

    pub fn save_state(&self, state: &CacheState) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
        write_private_atomic(&self.directory.join(STATE_FILE), &bytes)
    }

    /// The text of a cached copy, if the file exists and its SHA-256 is the one recorded.
    pub fn read_list(&self, list: &CachedList) -> Option<String> {
        if !is_cache_file_name(&list.file, ".txt") {
            return None;
        }
        let bytes = read_bounded(
            &self.directory.join(&list.file),
            crate::MAX_LIST_BYTES as u64,
        )?;
        if sha256_hex(&bytes) != list.sha256 {
            log::warn!("filter lists: the cached copy {} is damaged", list.file);
            return None;
        }
        String::from_utf8(bytes).ok()
    }

    /// Stores the text of an updated copy of list `id`; returns its file name and SHA-256.
    pub fn write_list(&self, id: &str, text: &str) -> io::Result<(String, String)> {
        if !is_safe_id(id) {
            return Err(io::Error::other("unsafe list id"));
        }
        let sha256 = sha256_hex(text.as_bytes());
        let file = format!("{id}-{}.txt", &sha256[..16]);
        write_private_atomic(&self.directory.join(&file), text.as_bytes())?;
        Ok((file, sha256))
    }

    /// The compiled engine with `key`, if the state names it and the file is intact.
    pub fn read_engine(&self, state: &CacheState, key: &str) -> Option<Vec<u8>> {
        let engine = state.engine.as_ref().filter(|engine| engine.key == key)?;
        if !is_cache_file_name(&engine.file, ".dat") {
            return None;
        }
        let bytes = read_bounded(&self.directory.join(&engine.file), MAX_ENGINE_BYTES)?;
        if sha256_hex(&bytes) != engine.sha256 {
            log::warn!("filter lists: the compiled engine {} is damaged", engine.file);
            return None;
        }
        Some(bytes)
    }

    /// Stores a compiled engine and returns its entry for the state.
    pub fn write_engine(&self, key: &str, bytes: &[u8]) -> io::Result<CachedEngine> {
        let sha256 = sha256_hex(bytes);
        let key_part = sha256_hex(key.as_bytes());
        let file = format!("engine-{}.dat", &key_part[..16]);
        write_private_atomic(&self.directory.join(&file), bytes)?;
        Ok(CachedEngine {
            file,
            key: key.to_owned(),
            sha256,
        })
    }

    /// Removes every file the state does not name: older copies and engines, and temporary
    /// files left by a write that was cut short. Call only while nothing else writes here.
    pub fn sweep(&self, state: &CacheState) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return 0;
        };
        let mut kept = state
            .lists
            .values()
            .map(|list| list.file.as_str())
            .collect::<Vec<_>>();
        kept.extend(state.engine.as_ref().map(|engine| engine.file.as_str()));
        kept.push(STATE_FILE);
        let mut removed = 0;
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let ours = is_cache_file_name(&name, ".txt")
                || is_cache_file_name(&name, ".dat")
                || (name.starts_with('.') && name.ends_with(".tmp"));
            let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
            if ours && is_file && !kept.contains(&name.as_str()) {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => removed += 1,
                    Err(error) => log::warn!("filter lists: could not remove {name}: {error}"),
                }
            }
        }
        removed
    }
}

/// The largest compiled engine read back, in bytes (the bundled lists compile to about 5 MB).
const MAX_ENGINE_BYTES: u64 = 64 * 1024 * 1024;

/// `<safe id>-<16 hex>.txt` or `engine-<16 hex>.dat`: the only names the cache writes.
fn is_cache_file_name(name: &str, extension: &str) -> bool {
    let Some(stem) = name.strip_suffix(extension) else {
        return false;
    };
    let Some((id, hash)) = stem.rsplit_once('-') else {
        return false;
    };
    is_safe_id(id)
        && hash.len() == 16
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The whole file if it is a regular file of at most `limit` bytes.
fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Cuts a message or validator from a server to a length worth keeping.
pub(crate) fn bounded(text: &str) -> String {
    let mut end = text.len().min(MAX_FIELD_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> (tempfile::TempDir, Cache) {
        let directory = tempfile::tempdir().expect("a directory");
        let cache = Cache::open(&directory.path().join("filter-lists")).expect("a cache");
        (directory, cache)
    }

    fn cached(cache: &Cache, id: &str, text: &str) -> CachedList {
        let (file, sha256) = cache.write_list(id, text).expect("written");
        CachedList {
            file,
            sha256,
            source_url: "https://example.org/list.txt".to_owned(),
            etag: Some("\"v2\"".to_owned()),
            last_modified: None,
            version: None,
            current_as_of: 1_794_775_611,
        }
    }

    #[test]
    fn the_directory_is_created_private() {
        let (_directory, cache) = cache();
        assert!(cache.directory().is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(cache.directory()).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn state_round_trips_and_missing_state_is_empty() {
        let (_directory, cache) = cache();
        assert_eq!(cache.load_state(), CacheState::default());

        let mut state = CacheState {
            last_check: Some(10),
            last_success: Some(11),
            ..CacheState::default()
        };
        state.lists.insert("easylist".to_owned(), cached(&cache, "easylist", "||a^\n"));
        cache.save_state(&state).expect("saved");

        assert_eq!(cache.load_state(), state);
    }

    #[test]
    fn damaged_state_reads_as_empty() {
        let (_directory, cache) = cache();
        for damage in [&b"{not json"[..], b"", b"\xff\xfe", br#"{"format_version": 99}"#] {
            std::fs::write(cache.directory().join(STATE_FILE), damage).unwrap();
            assert_eq!(cache.load_state(), CacheState::default(), "{damage:?}");
        }
    }

    #[test]
    fn state_naming_unsafe_files_is_not_trusted() {
        let (_directory, cache) = cache();
        let mut state = CacheState::default();
        let mut list = cached(&cache, "easylist", "||a^\n");
        list.file = "../../etc/passwd".to_owned();
        state.lists.insert("easylist".to_owned(), list);
        cache.save_state(&state).expect("saved");

        assert!(cache.load_state().lists.is_empty());
    }

    #[test]
    fn a_copy_is_read_back_only_while_it_is_intact() {
        let (_directory, cache) = cache();
        let list = cached(&cache, "easylist", "||ads.example^\n");
        assert_eq!(cache.read_list(&list).as_deref(), Some("||ads.example^\n"));

        std::fs::write(cache.directory().join(&list.file), "||ads.exam").unwrap();
        assert_eq!(cache.read_list(&list), None);

        std::fs::remove_file(cache.directory().join(&list.file)).unwrap();
        assert_eq!(cache.read_list(&list), None);
    }

    #[test]
    fn copies_are_named_after_their_content_and_replaced_atomically() {
        let (_directory, cache) = cache();
        let first = cached(&cache, "easylist", "one\n");
        let second = cached(&cache, "easylist", "two\n");

        assert_ne!(first.file, second.file);
        assert!(first.file.starts_with("easylist-") && first.file.ends_with(".txt"));
        // The old copy is still whole until the sweep removes it.
        assert_eq!(cache.read_list(&first).as_deref(), Some("one\n"));
        assert_eq!(cache.read_list(&second).as_deref(), Some("two\n"));
        // No temporary file is left behind.
        let names = std::fs::read_dir(cache.directory())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        assert!(names.iter().all(|name| !name.starts_with('.')), "{names:?}");
    }

    #[test]
    fn the_engine_is_read_back_only_for_its_key_and_while_intact() {
        let (_directory, cache) = cache();
        let mut state = CacheState::default();
        state.engine = Some(cache.write_engine("key-a", b"compiled").expect("written"));

        assert_eq!(cache.read_engine(&state, "key-a").as_deref(), Some(&b"compiled"[..]));
        assert_eq!(cache.read_engine(&state, "key-b"), None);

        let file = state.engine.as_ref().unwrap().file.clone();
        std::fs::write(cache.directory().join(file), b"compiles").unwrap();
        assert_eq!(cache.read_engine(&state, "key-a"), None);
    }

    #[test]
    fn the_sweep_keeps_what_the_state_names_and_removes_the_rest() {
        let (_directory, cache) = cache();
        let old = cached(&cache, "easylist", "old\n");
        let kept = cached(&cache, "easylist", "new\n");
        let mut state = CacheState::default();
        state.lists.insert("easylist".to_owned(), kept.clone());
        state.engine = Some(cache.write_engine("k", b"engine").unwrap());
        let stale_engine = cache.write_engine("other", b"old engine").unwrap();
        cache.save_state(&state).unwrap();
        std::fs::write(cache.directory().join(".state.json.123-0.tmp"), "cut short").unwrap();

        let removed = cache.sweep(&state);

        assert_eq!(removed, 3);
        assert!(!cache.directory().join(&old.file).exists());
        assert!(!cache.directory().join(&stale_engine.file).exists());
        assert_eq!(cache.read_list(&kept).as_deref(), Some("new\n"));
        assert!(cache.read_engine(&state, "k").is_some());
        assert_eq!(cache.load_state(), state);
    }
}
