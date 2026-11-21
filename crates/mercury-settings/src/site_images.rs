// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The file `site-images.json`: the sites whose images are turned off.
//!
//! ```json
//! {
//!   "version": 1,
//!   "blocked_hosts": ["example.com", "[::1]"]
//! }
//! ```
//!
//! It is a file of its own, not a field of [`Settings`](crate::Settings) (brief decision O12):
//! `settings.json` is changed by other lanes, and the per-site table moves into
//! `mercury-site-settings` later, which imports this file and deletes it.
//!
//! The store follows [`SettingsStore`](crate::SettingsStore): written privately and atomically
//! (`write_private_atomic`), read within [`MAX_SITE_IMAGES_FILE_BYTES`], a file that cannot be
//! understood moved aside to `site-images.json.damaged-<n>`, and a file from a newer version
//! ([`Loaded::Newer`]) left alone and never saved over. The version key is `version`, as the
//! brief froze it (the other profile files say `format_version`), so the reader here has its
//! own small version check instead of the shared one.
//!
//! # The key of a site
//!
//! An entry is the exact host of a page, in ASCII (punycode), lower case, without port or
//! trailing dot, an IPv6 literal in brackets. This crate does not depend on `talaria-core`
//! (which depends on this crate in its tests), so the rule that turns an entry into a key is
//! written here again, the same as `talaria_core::image_site_key` and its private entry rule;
//! both test files carry the same table of cases. Change one, change the other.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::atomic::write_private_atomic;
use crate::error::SettingsError;
use crate::read::{open_regular, short_reason};
use crate::settings::Repair;
use crate::store::{Loaded, move_aside};

/// The format version this crate writes into `site-images.json`.
pub const SITE_IMAGES_FORMAT_VERSION: u32 = 1;
/// The largest `site-images.json` that is read; a larger one is treated as damaged (unless its
/// first member says it is from a newer version).
pub const MAX_SITE_IMAGES_FILE_BYTES: u64 = 256 * 1024;
/// The most sites kept: the same number as `talaria_core::MAX_IMAGE_BLOCKED_SITES`.
pub const MAX_SITE_IMAGE_HOSTS: usize = 1000;

const SITE_IMAGES_FILE: &str = "site-images.json";
/// The field repaired when entries are dropped, normalised or reordered.
const HOSTS_FIELD: &str = "blocked_hosts";
/// How much of the start of a file the version check looks at when the file is too large to
/// read. A file this crate writes has its version within the first 20 bytes.
const PEEK_BYTES: usize = 256;
/// The longest key kept, in bytes: the longest name DNS allows (`talaria_core` has the same).
const MAX_KEY_BYTES: usize = 253;
/// The longest stored entry looked at before any parsing, in bytes.
const MAX_ENTRY_BYTES: usize = 1024;

/// What `site-images.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteImagesFile {
    /// The sites whose images are off, as keys (see the module text): sorted, without
    /// duplicates, at most [`MAX_SITE_IMAGE_HOSTS`]. [`SiteImagesFile::new`] and the store
    /// (on loading and saving) bring any list into that shape.
    pub blocked_hosts: Vec<String>,
}

impl SiteImagesFile {
    /// A file value from entries: each normalised to a key, invalid entries and duplicates
    /// dropped, the first [`MAX_SITE_IMAGE_HOSTS`] valid ones in the order given kept, then
    /// sorted.
    pub fn new(hosts: impl IntoIterator<Item = String>) -> Self {
        Self {
            blocked_hosts: normalised(hosts),
        }
    }

    /// The same value brought into the shape [`SiteImagesFile::new`] gives.
    #[must_use]
    pub fn normalised(self) -> Self {
        Self::new(self.blocked_hosts)
    }
}

/// The key of an entry: the text must be one host and nothing else (no scheme, port, path,
/// user, escape or space), and it is then normalised as the host of a page is. The twin of
/// `talaria_core::site_images::key_of_entry`; keep them the same.
fn key_of_entry(entry: &str) -> Option<String> {
    if entry.is_empty() || entry.len() > MAX_ENTRY_BYTES {
        return None;
    }
    // The URL parser strips tabs and newlines and decodes escapes, so those must be refused
    // before it sees the text: what it would make of them is not what the entry says.
    let plain = |c: char| {
        !(c.is_whitespace()
            || c.is_control()
            || matches!(c, '/' | '\\' | '?' | '#' | '@' | '%' | ':' | '[' | ']'))
    };
    let shaped = match entry
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        Some(literal) => literal
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.')),
        None => entry.chars().all(plain),
    };
    if !shaped {
        return None;
    }
    let url = Url::parse(&format!("https://{entry}/")).ok()?;
    // Whatever shape the entry had, it must have been consumed as a bare host.
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    // The key of a page: its host without the trailing dot, within the length limit. The `url`
    // crate has already made it lower case ASCII, decoded escapes and written IPv4 and IPv6
    // literals in their canonical forms.
    let key = url.host_str()?.trim_end_matches('.');
    (!key.is_empty() && key.len() <= MAX_KEY_BYTES).then(|| key.to_owned())
}

/// Entries to the stored list: keys, no invalid ones, no duplicates, the first
/// [`MAX_SITE_IMAGE_HOSTS`] in the order given, sorted.
fn normalised(hosts: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut kept = std::collections::BTreeSet::new();
    for host in hosts {
        if kept.len() >= MAX_SITE_IMAGE_HOSTS {
            break;
        }
        if let Some(key) = key_of_entry(&host) {
            kept.insert(key);
        }
    }
    kept.into_iter().collect()
}

/// What is on disk at the path.
#[derive(Debug)]
enum OnDisk {
    Missing,
    Damaged(String),
    /// The version found, which is above [`SITE_IMAGES_FORMAT_VERSION`].
    Newer(u32),
    /// The entries as the file lists them (strings only; anything else counted in `skipped`).
    Current {
        entries: Vec<String>,
        skipped: usize,
    },
}

/// The version at the very start of `bytes`: `{ "version": <digits>` followed by `,` or `}`.
/// Used only for a file too large to read, which is left alone when it says it is newer.
fn head_version(bytes: &[u8]) -> Option<u32> {
    let mut at = 0;
    let skip_space = |at: &mut usize| {
        while bytes
            .get(*at)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            *at += 1;
        }
    };
    skip_space(&mut at);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at += 1;
    skip_space(&mut at);
    let key = br#""version""#;
    if !bytes.get(at..)?.starts_with(key) {
        return None;
    }
    at += key.len();
    skip_space(&mut at);
    if bytes.get(at) != Some(&b':') {
        return None;
    }
    at += 1;
    skip_space(&mut at);
    let start = at;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    if at == start {
        return None;
    }
    let version = bytes[start..at].iter().fold(0_u32, |version, digit| {
        version
            .saturating_mul(10)
            .saturating_add(u32::from(digit - b'0'))
    });
    skip_space(&mut at);
    matches!(bytes.get(at), Some(b',' | b'}')).then_some(version)
}

/// The version member of a decoded document: absent is 0 (a hand-written file in the version 1
/// layout), a non-negative integer saturates to `u32::MAX`, anything else is no version.
fn document_version(document: &serde_json::Map<String, Value>) -> Result<u32, String> {
    let Some(value) = document.get("version") else {
        return Ok(0);
    };
    let not_a_version = || "version is not a version number".to_owned();
    let Value::Number(number) = value else {
        return Err(not_a_version());
    };
    if let Some(version) = number.as_u64() {
        return Ok(u32::try_from(version).unwrap_or(u32::MAX));
    }
    // Integers beyond u64 arrive as floats.
    match number.as_f64() {
        Some(float) if float >= u64::MAX as f64 && float.fract() == 0.0 => Ok(u32::MAX),
        _ => Err(not_a_version()),
    }
}

/// Reads and classifies the file at `path`, within [`MAX_SITE_IMAGES_FILE_BYTES`].
fn read_file(path: &Path) -> Result<OnDisk, SettingsError> {
    let Some((file, _length)) = open_regular(path)? else {
        return Ok(OnDisk::Missing);
    };
    let mut bytes = Vec::new();
    file.take(MAX_SITE_IMAGES_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| SettingsError::io("read", path, error))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SITE_IMAGES_FILE_BYTES {
        let head = &bytes[..PEEK_BYTES.min(bytes.len())];
        return Ok(match head_version(head) {
            Some(version) if version > SITE_IMAGES_FORMAT_VERSION => OnDisk::Newer(version),
            _ => OnDisk::Damaged(format!("larger than {MAX_SITE_IMAGES_FILE_BYTES} bytes")),
        });
    }

    let document = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(document)) => document,
        Ok(_) => return Ok(OnDisk::Damaged("not a JSON object".to_owned())),
        Err(error) => return Ok(OnDisk::Damaged(short_reason(error.to_string()))),
    };
    match document_version(&document) {
        Err(reason) => return Ok(OnDisk::Damaged(reason)),
        Ok(version) if version > SITE_IMAGES_FORMAT_VERSION => return Ok(OnDisk::Newer(version)),
        Ok(_) => {}
    }
    Ok(match document.get("blocked_hosts") {
        None | Some(Value::Null) => OnDisk::Current {
            entries: Vec::new(),
            skipped: 0,
        },
        Some(Value::Array(items)) => {
            let mut entries = Vec::with_capacity(items.len());
            let mut skipped = 0;
            for item in items {
                match item {
                    Value::String(text) => entries.push(text.clone()),
                    _ => skipped += 1,
                }
            }
            OnDisk::Current { entries, skipped }
        }
        Some(_) => OnDisk::Damaged("blocked_hosts is not a list".to_owned()),
    })
}

/// Reads and writes `<profile>/site-images.json`.
#[derive(Clone, Debug)]
pub struct SiteImagesStore {
    path: PathBuf,
}

impl SiteImagesStore {
    /// A store for `<profile_dir>/site-images.json`. Touches nothing on disk.
    pub fn new(profile_dir: &Path) -> Self {
        Self {
            path: profile_dir.join(SITE_IMAGES_FILE),
        }
    }

    /// Reads the file, its entries normalised, deduplicated, capped and sorted (see
    /// [`SiteImagesFile::new`]); the load reports one [`Repair`] when that changed the list.
    ///
    /// A file from a newer version is left untouched and reported as [`Loaded::Newer`]
    /// (whatever its size, when its first member says so). A file over
    /// [`MAX_SITE_IMAGES_FILE_BYTES`], or one that is not a JSON object with a list of hosts,
    /// is moved aside and reported as [`Loaded::Damaged`]. A directory or other non-file at the
    /// path, or a file that cannot be read, is an error.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Io`] as described above.
    pub fn load(&self) -> Result<Loaded<SiteImagesFile>, SettingsError> {
        match read_file(&self.path)? {
            OnDisk::Missing => Ok(Loaded::Missing(SiteImagesFile::default())),
            OnDisk::Current { entries, skipped } => {
                let found = entries.len() + skipped;
                let kept = normalised(entries.iter().cloned());
                let repairs = if skipped == 0 && kept == entries {
                    Vec::new()
                } else {
                    log::info!(
                        "site images: kept {} of {found} entries of site-images.json",
                        kept.len()
                    );
                    vec![Repair {
                        field: HOSTS_FIELD,
                        reason: format!(
                            "entries were dropped, normalised or sorted (kept {} of {found})",
                            kept.len()
                        ),
                    }]
                };
                Ok(Loaded::Ok {
                    value: SiteImagesFile {
                        blocked_hosts: kept,
                    },
                    repairs,
                })
            }
            OnDisk::Newer(found_version) => {
                log::warn!(
                    "{} was written by a newer version (format {found_version}); using none and \
                     not saving",
                    self.path.display()
                );
                Ok(Loaded::Newer {
                    value: SiteImagesFile::default(),
                    found_version,
                })
            }
            OnDisk::Damaged(reason) => {
                let backup = move_aside(&self.path)?;
                log::warn!(
                    "{} could not be understood ({reason}); moved to {}; using none",
                    self.path.display(),
                    backup.display()
                );
                Ok(Loaded::Damaged {
                    value: SiteImagesFile::default(),
                    backup,
                })
            }
        }
    }

    /// Writes the file, its entries normalised, privately and atomically.
    ///
    /// Refused with [`SettingsError::NewerVersion`], changing nothing, when the file on disk was
    /// written by a newer version. A file there that cannot be understood is moved aside first.
    /// A symlink at the path is replaced, not written through.
    ///
    /// # Errors
    ///
    /// [`SettingsError::NewerVersion`] as above; [`SettingsError::Io`] when reading what is
    /// there or writing fails; [`SettingsError::TooLarge`] if the file would be over
    /// [`MAX_SITE_IMAGES_FILE_BYTES`], which no normalised list can do (a thousand names of 253
    /// bytes make about 261,000 of the 262,144 bytes allowed; a test pins that), so the check
    /// only guards a change of the limits.
    pub fn save(&self, file: &SiteImagesFile) -> Result<(), SettingsError> {
        #[derive(Serialize)]
        struct Document<'a> {
            version: u32,
            blocked_hosts: &'a [String],
        }

        let hosts = normalised(file.blocked_hosts.iter().cloned());
        let mut bytes = serde_json::to_vec_pretty(&Document {
            version: SITE_IMAGES_FORMAT_VERSION,
            blocked_hosts: &hosts,
        })
        .map_err(|source| SettingsError::Serialize {
            what: "site images",
            source,
        })?;
        bytes.push(b'\n');
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > MAX_SITE_IMAGES_FILE_BYTES {
            return Err(SettingsError::TooLarge {
                what: "site images",
                size,
                limit: MAX_SITE_IMAGES_FILE_BYTES,
            });
        }

        match read_file(&self.path)? {
            OnDisk::Missing | OnDisk::Current { .. } => {}
            OnDisk::Newer(found) => {
                return Err(SettingsError::NewerVersion {
                    path: self.path.clone(),
                    found,
                    supported: SITE_IMAGES_FORMAT_VERSION,
                });
            }
            OnDisk::Damaged(reason) => {
                let backup = move_aside(&self.path)?;
                log::warn!(
                    "{} could not be understood ({reason}); kept at {} before replacing it",
                    self.path.display(),
                    backup.display()
                );
            }
        }
        write_private_atomic(&self.path, &bytes)
            .map_err(|error| SettingsError::io("write", &self.path, error))
    }
}
