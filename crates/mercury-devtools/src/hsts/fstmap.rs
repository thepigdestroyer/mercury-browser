// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The HSTS preload data file: written, read, verified and replaced safely.
//!
//! The file is an [`fst`] map (Servo's `HstsPreloadList`): the key is a host name, the value is
//! `2 * rank + include_subdomains`, where `rank` is the 0-based position of the key in key
//! (byte) order. Servo reads only the parity of the value ("the FST map ids were constructed such
//! that the parity represents the includeSubdomain flag", `third_party/servo-net/hsts.rs`); the
//! rank part is what the script that made Servo's file wrote, and what [`build`] writes, so that
//! the same list always gives the same bytes. [`Preload::open`] accepts any file Servo can read
//! and that also carries a checksum; [`Preload::id_scheme_violation`] says whether the ranks hold.
//!
//! Servo fails open: when it cannot read the file it logs one line and uses an empty list, so a
//! damaged file would switch the preload off without anyone noticing. That is why [`Preload::open`]
//! checks what Servo's `Map::new` does not: the FST's checksum, and that every key is a plain
//! lower-case host name.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fst::{Map, MapBuilder, Streamer};

use super::Entry;
use super::source::valid_host;

/// The value written for a key: `2 * rank + include_subdomains`.
fn id_for(rank: usize, include_subdomains: bool) -> u64 {
    2 * rank as u64 + u64::from(include_subdomains)
}

/// Whether a value carries the include-subdomains flag (the parity Servo reads).
fn includes_subdomains(value: u64) -> bool {
    value % 2 == 1
}

/// Writes the data file for `entries`, in any order.
///
/// The entries are sorted by host and written with the value `2 * rank + include_subdomains`, so
/// the bytes depend only on the set of entries, never on the order they came in. A host that is
/// listed twice is an error (merging duplicates is the reader's job, see `source::parse_chromium`),
/// and so is a host that is not already a lower-case name that [`valid_host`] accepts.
pub fn build(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut sorted: BTreeMap<&str, bool> = BTreeMap::new();
    for entry in entries {
        check_key(&entry.host).map_err(|why| format!("cannot write the HSTS list: {why}"))?;
        if sorted
            .insert(entry.host.as_str(), entry.include_subdomains)
            .is_some()
        {
            return Err(format!(
                "cannot write the HSTS list: the host {:?} is listed more than once",
                entry.host
            ));
        }
    }
    let mut builder = MapBuilder::memory();
    for (rank, (host, include_subdomains)) in sorted.iter().enumerate() {
        builder
            .insert(host, id_for(rank, *include_subdomains))
            .map_err(|error| format!("cannot write the HSTS list: {error}"))?;
    }
    builder
        .into_inner()
        .map_err(|error| format!("cannot write the HSTS list: {error}"))
}

/// A key of the data file: a host name [`valid_host`] accepts that is already in lower case.
/// The error names the key.
fn check_key(key: &str) -> Result<(), String> {
    let lower = valid_host(key)?;
    if lower != key {
        return Err(format!(
            "host name {key:?} is not lower-case (it must be {lower:?})"
        ));
    }
    Ok(())
}

/// A data file that was opened and verified: the list Servo would read.
#[derive(Clone)]
pub struct Preload {
    map: Map<Vec<u8>>,
    include_subdomains: usize,
}

impl fmt::Debug for Preload {
    /// Counts only: the map itself is megabytes of bytes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Preload")
            .field("entries", &self.len())
            .field("include_subdomains", &self.include_subdomains)
            .finish()
    }
}

impl Preload {
    /// Opens the bytes of a data file and checks them.
    ///
    /// Beyond Servo's own `Map::new` (header, version, footer) this verifies the FST checksum,
    /// which `Map::new` does not (a flipped byte inside the file would otherwise load and
    /// answer wrongly), and scans every key: it must be UTF-8, equal to its own lower-cased form
    /// and a name [`valid_host`] accepts. The error names the first bad key.
    ///
    /// It accepts every file Servo can read that has a checksum, so it does **not** require the
    /// `2 * rank + flag` value scheme: see [`Preload::id_scheme_violation`]. A file written by
    /// `fst` before 0.4 (format versions 1 and 2) has no checksum and is refused for that.
    pub fn open(bytes: Vec<u8>) -> Result<Preload, String> {
        let map = Map::new(bytes).map_err(|error| format!("not an fst map file: {error}"))?;
        map.as_fst().verify().map_err(|error| {
            format!("the file is damaged or has no checksum (the checksum is verified): {error}")
        })?;
        let mut include_subdomains = 0;
        let mut stream = map.stream();
        let mut number = 0_usize;
        while let Some((key, value)) = stream.next() {
            number += 1;
            let key = std::str::from_utf8(key).map_err(|_| {
                format!(
                    "the file has a key that is not UTF-8: {:?} (key number {number})",
                    String::from_utf8_lossy(key)
                )
            })?;
            check_key(key).map_err(|why| {
                format!("the file has an invalid key (key number {number}): {why}")
            })?;
            if includes_subdomains(value) {
                include_subdomains += 1;
            }
        }
        drop(stream);
        Ok(Preload {
            map,
            include_subdomains,
        })
    }

    /// The number of hosts.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The number of hosts that include their subdomains (odd values).
    pub fn include_subdomains_count(&self) -> usize {
        self.include_subdomains
    }

    /// Every entry, in key order.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::with_capacity(self.len());
        let mut stream = self.map.stream();
        while let Some((key, value)) = stream.next() {
            entries.push(Entry {
                host: String::from_utf8_lossy(key).into_owned(),
                include_subdomains: includes_subdomains(value),
            });
        }
        entries
    }

    /// The include-subdomains flag of exactly this host name, or `None` when it is not listed.
    pub fn get(&self, host: &str) -> Option<bool> {
        self.map.get(host).map(includes_subdomains)
    }

    /// Whether the list covers `host`: the host itself is listed (whatever its flag), or a parent
    /// domain is listed with `include_subdomains`. Parents are tried longest first, down to the
    /// last label. `host` must already be lower case without a trailing dot, as the host of a URL
    /// is; anything else is not found.
    ///
    /// This is what an entry means (and what Chromium does), written as Servo's
    /// `HstsPreloadList::is_host_secure` is, but **it is not Servo's walk in one case**: Servo
    /// starts at the registrable domain of the host (`reg_suffix` in
    /// `third_party/servo-net-traits/pub_domains.rs`: the public suffix plus one label) and only
    /// adds labels from there, so it never looks at a listed entry that is itself a public suffix.
    /// For the entries `dev` or `app` Servo says yes to the bare host `dev` and no to
    /// `example.dev`; this method says yes to both. For every listed name that is not a public
    /// suffix the two agree. In the list Servo 0.8.0 ships, 75 of the 166,958 entries are public
    /// suffixes: the 51 one-label entries (`dev`, `app`, `google`, ...) and 24 more (`appspot.com`,
    /// `blogspot.com`, `now.sh`, ...); the tests count them. `hsts-preload` uses this method for
    /// its own checks (the canaries, `check`, `diff`), never to predict the browser to the byte.
    pub fn is_secure(&self, host: &str) -> bool {
        let mut candidate = host;
        loop {
            if let Some(value) = self.map.get(candidate)
                && (includes_subdomains(value) || candidate == host)
            {
                return true;
            }
            match candidate.split_once('.') {
                Some((_, parent)) => candidate = parent,
                None => return false,
            }
        }
    }

    /// The first key whose value is not `2 * rank + flag` (what [`build`] writes and Servo's
    /// script wrote), as a sentence; `None` when every key follows the scheme. Servo itself reads
    /// only the parity, so a breach does not stop the file from working.
    pub fn id_scheme_violation(&self) -> Option<String> {
        let mut stream = self.map.stream();
        let mut rank = 0_usize;
        while let Some((key, value)) = stream.next() {
            let expected = id_for(rank, includes_subdomains(value));
            if value != expected {
                return Some(format!(
                    "the host {:?} (position {rank} in key order, counting from 0) has the value \
                     {value}, but the scheme `2 * position + include_subdomains` gives {expected}",
                    String::from_utf8_lossy(key)
                ));
            }
            rank += 1;
        }
        None
    }

    /// Whether [`build`] given this file's own entries writes exactly `original`: the file is
    /// what this tool would have written (same `fst` encoder, same value scheme). A file that is
    /// valid but not identical is fine for Servo; `check` reports it as a note only.
    pub fn rebuilds_identically(&self, original: &[u8]) -> bool {
        build(&self.entries()).is_ok_and(|rebuilt| rebuilt == original)
    }
}

/// Distinguishes the temporary files of one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replaces `path` with `bytes` so that a reader sees the old file or the new one, never half a
/// file: the bytes go to a temporary file in the same directory (the same file system, so the
/// rename cannot copy), which is flushed to disk and renamed over `path`.
///
/// The directory of `path` must already exist (it is not created). On any error the temporary
/// file is removed and an existing `path` is left as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} does not name a file", path.display()))?;
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if !directory.is_dir() {
        return Err(format!(
            "cannot write {}: the directory {} does not exist",
            path.display(),
            directory.display()
        ));
    }

    let (temp_path, mut temp_file) = create_temp(directory, &name.to_string_lossy())
        .map_err(|error| format!("cannot write next to {}: {error}", path.display()))?;
    let written = temp_file
        .write_all(bytes)
        .and_then(|()| temp_file.sync_all())
        .map_err(|error| format!("cannot write {}: {error}", temp_path.display()));
    drop(temp_file);
    let result = written.and_then(|()| {
        std::fs::rename(&temp_path, path).map_err(|error| {
            format!(
                "cannot replace {} with {}: {error}",
                path.display(),
                temp_path.display()
            )
        })
    });
    if result.is_err() {
        // Best effort: when the removal fails too there is nothing more to do.
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

/// Creates a new, empty temporary file next to the target (`create_new`, so it never opens or
/// truncates a file that is already there).
fn create_temp(directory: &Path, name: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    let mut attempt = 0;
    loop {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_path = directory.join(format!(".{name}.{}-{counter}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => {
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}
