// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The zip archive of a package, read with size caps and with paths checked before anything is written.
//!
//! A package (`.crx`, `.xpi`, `.zip`) is untrusted input. [`ZipPackage::open`] reads and validates the
//! whole central directory up front, before a single byte is inflated:
//!
//! - the number of entries, the length and depth of every name, and the names themselves (no `..`, `.`,
//!   empty, absolute, drive-letter, backslash, NUL, control or bidirectional-control components: such a
//!   name is refused, never rewritten);
//! - duplicate names, names that collide when case is ignored (macOS volumes are case-insensitive), and a
//!   file and a directory with the same path;
//! - symbolic links and every other non-regular entry, encrypted entries, and every compression method
//!   other than stored and deflate;
//! - the declared sizes against the [`Limits`], and the compression-ratio rule.
//!
//! Declared sizes in a zip directory are never trusted. [`ZipPackage::read_file`],
//! [`ZipPackage::visit_file`] and [`ZipPackage::extract_to`] inflate through a reader that stops one byte
//! past the declared size, count the bytes that were really produced, and fail when they differ from the
//! declaration or exceed a cap.
//!
//! Normalisation differences between file systems (for example Unicode NFC against NFD on macOS) are not
//! folded when names are compared; [`ZipPackage::extract_to`] creates every file with `create_new`, so
//! two names that the volume treats as one fail the extraction instead of overwriting each other.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zip::CompressionMethod;
use zip::ZipArchive;
use zip::read::{ArchiveOffset, Config};
use zip::result::ZipError;

/// The size caps of this crate (decision D12). Every default is tested at its edge and one over.
///
/// The one `Limits` of the crate: the manifest parser takes the same struct (`max_manifest_bytes`,
/// `max_json_depth`), and the signature code uses `max_signature_file_bytes` and `max_certificates`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// The package file: 200 MiB.
    pub max_package_bytes: u64,
    /// Entries (files and directories) in the archive: 20,000.
    pub max_entries: usize,
    /// One entry, uncompressed: 100 MiB.
    pub max_entry_bytes: u64,
    /// All entries together, uncompressed: 500 MiB.
    pub max_total_bytes: u64,
    /// An entry above `ratio_min_entry_bytes` whose uncompressed size is more than this many times its
    /// compressed size is refused: 200.
    pub max_compression_ratio: u64,
    /// The ratio rule applies only to entries above this size: 1 MiB.
    pub ratio_min_entry_bytes: u64,
    /// An entry name, in bytes: 512.
    pub max_path_bytes: usize,
    /// Components in an entry name: 32.
    pub max_path_depth: usize,
    /// `manifest.json`: 1 MiB.
    pub max_manifest_bytes: u64,
    /// Nesting of arrays and objects in a JSON document: 64.
    pub max_json_depth: usize,
    /// One signature file (`META-INF/cose.sig`, `cose.manifest`, `mozilla.rsa`, `mozilla.sf`,
    /// `manifest.mf`): 1 MiB.
    pub max_signature_file_bytes: u64,
    /// Certificates read from one signature: 8.
    pub max_certificates: usize,
}

impl Limits {
    /// The defaults, as a constant.
    pub const DEFAULT: Limits = Limits {
        max_package_bytes: 200 * 1024 * 1024,
        max_entries: 20_000,
        max_entry_bytes: 100 * 1024 * 1024,
        max_total_bytes: 500 * 1024 * 1024,
        max_compression_ratio: 200,
        ratio_min_entry_bytes: 1024 * 1024,
        max_path_bytes: 512,
        max_path_depth: 32,
        max_manifest_bytes: 1024 * 1024,
        max_json_depth: 64,
        max_signature_file_bytes: 1024 * 1024,
        max_certificates: 8,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Limits::DEFAULT
    }
}

/// Why an entry name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NameProblem {
    /// The name is empty.
    #[error("the name is empty")]
    Empty,
    /// The name is longer than `Limits::max_path_bytes`.
    #[error("the name is too long")]
    TooLong,
    /// The name has more components than `Limits::max_path_depth`.
    #[error("the name is nested too deeply")]
    TooDeep,
    /// The name is not valid UTF-8.
    #[error("the name is not valid UTF-8")]
    NotUtf8,
    /// The name starts with `/`.
    #[error("the name is absolute")]
    Absolute,
    /// The name starts with a drive letter such as `C:`.
    #[error("the name starts with a drive letter")]
    DriveLetter,
    /// The name has a backslash.
    #[error("the name has a backslash")]
    Backslash,
    /// The name has a NUL byte.
    #[error("the name has a NUL byte")]
    Nul,
    /// The name has a control character or a bidirectional-control character.
    #[error("the name has a control character")]
    ControlCharacter,
    /// A component is `..`.
    #[error("the name has a `..` component")]
    ParentComponent,
    /// A component is `.`.
    #[error("the name has a `.` component")]
    CurrentComponent,
    /// A component is empty (`a//b`).
    #[error("the name has an empty component")]
    EmptyComponent,
}

/// Why a package or an entry was refused or could not be read. No variant holds file contents or key
/// bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveError {
    /// The package is larger than `Limits::max_package_bytes`.
    #[error("the package is {found} bytes, over the limit of {limit} bytes")]
    PackageTooLarge {
        /// The limit.
        limit: u64,
        /// The size found.
        found: u64,
    },
    /// The bytes are not a zip archive, or the archive is damaged or truncated.
    #[error("the package is not a readable zip archive")]
    Malformed,
    /// The archive has more entries than `Limits::max_entries`.
    #[error("the archive has {found} entries, over the limit of {limit}")]
    TooManyEntries {
        /// The limit.
        limit: usize,
        /// The number of entries found.
        found: usize,
    },
    /// An entry name is unsafe.
    #[error("entry {index} has an unsafe name: {reason}")]
    UnsafeName {
        /// Position in the directory.
        index: usize,
        /// What is wrong.
        reason: NameProblem,
    },
    /// Two entries have the same name.
    #[error("entry {index} repeats the name of another entry")]
    DuplicateName {
        /// Position in the directory.
        index: usize,
    },
    /// Two entries differ only in letter case.
    #[error("entry {index} differs from another entry only in letter case")]
    CaseCollision {
        /// Position in the directory.
        index: usize,
    },
    /// A name is both a file and a directory (`a` and `a/b`).
    #[error("entry {index} is both a file and a directory")]
    PathConflict {
        /// Position in the directory.
        index: usize,
    },
    /// An entry is a symbolic link.
    #[error("entry {index} is a symbolic link")]
    SymlinkEntry {
        /// Position in the directory.
        index: usize,
    },
    /// An entry is neither a regular file nor a directory.
    #[error("entry {index} is not a regular file or a directory")]
    SpecialEntry {
        /// Position in the directory.
        index: usize,
    },
    /// An entry is encrypted.
    #[error("entry {index} is encrypted")]
    Encrypted {
        /// Position in the directory.
        index: usize,
    },
    /// An entry uses a compression method other than stored or deflate.
    #[error("entry {index} uses unsupported compression method {method}")]
    UnsupportedCompression {
        /// Position in the directory.
        index: usize,
        /// The method number of the zip format.
        method: u16,
    },
    /// An entry is larger than the cap.
    #[error("entry {index} is over the limit of {limit} bytes")]
    EntryTooLarge {
        /// Position in the directory.
        index: usize,
        /// The cap that applied.
        limit: u64,
    },
    /// All entries together are larger than the cap.
    #[error("the entries together are over the limit of {limit} bytes")]
    TotalTooLarge {
        /// The cap.
        limit: u64,
    },
    /// An entry compresses by more than the allowed ratio.
    #[error("entry {index} compresses by more than the allowed ratio")]
    CompressionRatio {
        /// Position in the directory.
        index: usize,
    },
    /// An entry inflated to a different size than the directory declares.
    #[error("entry {index} does not have the size the directory declares")]
    SizeMismatch {
        /// Position in the directory.
        index: usize,
    },
    /// There is no entry of that name.
    #[error("there is no such entry")]
    NotFound,
    /// The name asks for a directory, not a file.
    #[error("the entry is a directory")]
    IsDirectory,
    /// `extract_to` refuses a destination that exists and is not an empty directory.
    #[error("the destination exists and is not an empty directory")]
    DestinationNotEmpty,
    /// A path resolved to a place outside the destination.
    #[error("an entry would be written outside the destination")]
    PathEscape,
    /// Reading or writing failed.
    #[error("input/output error: {0:?}")]
    Io(io::ErrorKind),
}

fn map_zip_error(error: ZipError) -> ArchiveError {
    match error {
        ZipError::Io(e) => ArchiveError::Io(e.kind()),
        _ => ArchiveError::Malformed,
    }
}

/// An entry of a package, as the validated directory declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
    /// The validated name. A directory ends with `/`.
    pub name: String,
    /// The declared uncompressed size.
    pub size: u64,
    /// The declared compressed size.
    pub compressed_size: u64,
    /// A directory entry.
    pub is_dir: bool,
    /// Whether the entry is marked executable (any execute bit in the Unix mode).
    pub executable: bool,
}

type Source = Cursor<Arc<[u8]>>;

/// A zip archive whose directory has been validated against [`Limits`].
///
/// Reading takes `&self`; the archive is read by one caller at a time.
pub struct ZipPackage {
    archive: Mutex<ZipArchive<Source>>,
    entries: Vec<PackageEntry>,
    by_name: HashMap<String, usize>,
    limits: Limits,
}

impl std::fmt::Debug for ZipPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipPackage")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// What [`ZipPackage::extract_to`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExtractStats {
    /// Regular files written.
    pub files: usize,
    /// Directories created (explicit entries and parents).
    pub directories: usize,
    /// Bytes written.
    pub bytes: u64,
}

const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

/// Checks one entry name against the rules of this module and returns whether it names a directory
/// (a trailing `/`). The name is never changed.
pub fn validate_entry_name(raw: &[u8], limits: &Limits) -> Result<bool, NameProblem> {
    if raw.is_empty() {
        return Err(NameProblem::Empty);
    }
    if raw.len() > limits.max_path_bytes {
        return Err(NameProblem::TooLong);
    }
    if raw.contains(&0) {
        return Err(NameProblem::Nul);
    }
    let name = std::str::from_utf8(raw).map_err(|_| NameProblem::NotUtf8)?;
    if name.contains('\\') {
        return Err(NameProblem::Backslash);
    }
    if name.chars().any(is_forbidden_char) {
        return Err(NameProblem::ControlCharacter);
    }
    if name.starts_with('/') {
        return Err(NameProblem::Absolute);
    }
    let bytes = name.as_bytes();
    if let (Some(first), Some(b':')) = (bytes.first(), bytes.get(1))
        && first.is_ascii_alphabetic()
    {
        return Err(NameProblem::DriveLetter);
    }
    let is_dir = name.ends_with('/');
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    let mut depth = 0usize;
    for component in trimmed.split('/') {
        match component {
            "" => return Err(NameProblem::EmptyComponent),
            "." => return Err(NameProblem::CurrentComponent),
            ".." => return Err(NameProblem::ParentComponent),
            _ => {}
        }
        depth = depth.saturating_add(1);
    }
    if depth > limits.max_path_depth {
        return Err(NameProblem::TooDeep);
    }
    Ok(is_dir)
}

fn is_forbidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

impl ZipPackage {
    /// Opens a zip archive that starts at the first byte of `bytes` (a prefix is refused) and validates
    /// its directory. Nothing is inflated.
    pub fn open(bytes: impl Into<Arc<[u8]>>, limits: &Limits) -> Result<ZipPackage, ArchiveError> {
        ZipPackage::open_at(bytes, 0, limits)
    }

    /// Like [`ZipPackage::open`] for an archive that starts at `archive_offset`: the CRX case, where a
    /// verified header precedes the zip.
    pub fn open_at(
        bytes: impl Into<Arc<[u8]>>,
        archive_offset: u64,
        limits: &Limits,
    ) -> Result<ZipPackage, ArchiveError> {
        let bytes: Arc<[u8]> = bytes.into();
        let found = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if found > limits.max_package_bytes {
            return Err(ArchiveError::PackageTooLarge {
                limit: limits.max_package_bytes,
                found,
            });
        }
        if archive_offset > found {
            return Err(ArchiveError::Malformed);
        }
        let config = Config {
            archive_offset: ArchiveOffset::Known(archive_offset),
        };
        let mut archive =
            ZipArchive::with_config(config, Cursor::new(bytes)).map_err(map_zip_error)?;
        if archive.has_duplicate_names() {
            return Err(ArchiveError::DuplicateName { index: 0 });
        }
        if archive.len() > limits.max_entries {
            return Err(ArchiveError::TooManyEntries {
                limit: limits.max_entries,
                found: archive.len(),
            });
        }
        let (entries, by_name) = Self::validate_directory(&archive, limits)?;
        archive
            .validate_header_consistency()
            .map_err(|_| ArchiveError::Malformed)?;
        Ok(ZipPackage {
            archive: Mutex::new(archive),
            entries,
            by_name,
            limits: limits.clone(),
        })
    }

    fn validate_directory(
        archive: &ZipArchive<Source>,
        limits: &Limits,
    ) -> Result<(Vec<PackageEntry>, HashMap<String, usize>), ArchiveError> {
        let mut entries = Vec::with_capacity(archive.len());
        let mut by_name: HashMap<String, usize> = HashMap::with_capacity(archive.len());
        let mut folded: HashSet<String> = HashSet::with_capacity(archive.len());
        let mut file_paths: HashSet<String> = HashSet::new();
        let mut dir_paths: HashSet<String> = HashSet::new();
        let mut total: u64 = 0;
        for index in 0..archive.len() {
            let entry = archive.by_index_data(index).map_err(map_zip_error)?;
            let is_dir = validate_entry_name(entry.name_raw(), limits)
                .map_err(|reason| ArchiveError::UnsafeName { index, reason })?;
            let name = String::from_utf8_lossy(entry.name_raw()).into_owned();
            if entry.encrypted() {
                return Err(ArchiveError::Encrypted { index });
            }
            match entry.compression() {
                CompressionMethod::Stored | CompressionMethod::Deflated => {}
                other => {
                    // `to_u16` is the only public way to the method number of the zip format.
                    #[allow(deprecated)]
                    let method = other.to_u16();
                    return Err(ArchiveError::UnsupportedCompression { index, method });
                }
            }
            let mode = entry.external_attributes() >> 16;
            let kind = mode & S_IFMT;
            if kind == S_IFLNK {
                return Err(ArchiveError::SymlinkEntry { index });
            }
            let kind_ok = if is_dir {
                kind == 0 || kind == S_IFDIR
            } else {
                kind == 0 || kind == S_IFREG
            };
            if !kind_ok {
                return Err(ArchiveError::SpecialEntry { index });
            }
            if by_name.contains_key(&name) {
                return Err(ArchiveError::DuplicateName { index });
            }
            if !folded.insert(name.to_lowercase()) {
                return Err(ArchiveError::CaseCollision { index });
            }
            let path = name.strip_suffix('/').unwrap_or(&name).to_string();
            let conflict = if is_dir {
                file_paths.contains(&path)
            } else {
                dir_paths.contains(&path)
            };
            if conflict {
                return Err(ArchiveError::PathConflict { index });
            }
            let mut prefix = String::new();
            let mut parts = path.split('/').peekable();
            while let Some(part) = parts.next() {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                let is_last = parts.peek().is_none();
                if is_last && !is_dir {
                    break;
                }
                if file_paths.contains(&prefix) {
                    return Err(ArchiveError::PathConflict { index });
                }
                dir_paths.insert(prefix.clone());
            }
            if !is_dir {
                file_paths.insert(path);
            }
            let size = entry.size();
            let compressed_size = entry.compressed_size();
            if size > limits.max_entry_bytes {
                return Err(ArchiveError::EntryTooLarge {
                    index,
                    limit: limits.max_entry_bytes,
                });
            }
            if ratio_exceeded(size, compressed_size, limits) {
                return Err(ArchiveError::CompressionRatio { index });
            }
            total = total.saturating_add(size);
            if total > limits.max_total_bytes {
                return Err(ArchiveError::TotalTooLarge {
                    limit: limits.max_total_bytes,
                });
            }
            by_name.insert(name.clone(), index);
            entries.push(PackageEntry {
                name,
                size,
                compressed_size,
                is_dir,
                executable: mode & 0o111 != 0,
            });
        }
        Ok((entries, by_name))
    }

    fn lock(&self) -> MutexGuard<'_, ZipArchive<Source>> {
        self.archive.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The validated entries, in directory order.
    pub fn entries(&self) -> &[PackageEntry] {
        &self.entries
    }

    /// The names of the validated entries, in directory order (directories end with `/`).
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// The entry of that exact name.
    pub fn entry(&self, name: &str) -> Option<&PackageEntry> {
        self.by_name.get(name).and_then(|i| self.entries.get(*i))
    }

    /// Whether an entry of that exact name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// The limits this package was opened with.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Reads one file into memory, at most `Limits::max_entry_bytes`.
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>, ArchiveError> {
        self.read_file_capped(name, self.limits.max_entry_bytes)
    }

    /// Reads one file into memory, refusing it before inflating when its declared size is over `cap`.
    pub fn read_file_capped(&self, name: &str, cap: u64) -> Result<Vec<u8>, ArchiveError> {
        let mut out = Vec::new();
        self.visit_file(name, cap, &mut |chunk| out.extend_from_slice(chunk))?;
        Ok(out)
    }

    /// Inflates one file and hands the bytes to `sink` in chunks. Returns the number of bytes. The
    /// declared size must be at most `cap`, and the bytes actually inflated must equal the declared
    /// size.
    pub fn visit_file(
        &self,
        name: &str,
        cap: u64,
        sink: &mut dyn FnMut(&[u8]),
    ) -> Result<u64, ArchiveError> {
        let index = *self.by_name.get(name).ok_or(ArchiveError::NotFound)?;
        self.stream_entry(index, cap, &mut |chunk| {
            sink(chunk);
            Ok(())
        })
    }

    fn stream_entry(
        &self,
        index: usize,
        cap: u64,
        sink: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<u64, ArchiveError> {
        let entry = self.entries.get(index).ok_or(ArchiveError::NotFound)?;
        if entry.is_dir {
            return Err(ArchiveError::IsDirectory);
        }
        let declared = entry.size;
        if declared > cap {
            return Err(ArchiveError::EntryTooLarge { index, limit: cap });
        }
        let mut archive = self.lock();
        let file = archive.by_index(index).map_err(map_zip_error)?;
        let mut reader = file.take(declared.saturating_add(1));
        let mut buffer = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(ArchiveError::Io(e.kind())),
            };
            total = total.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
            if total > declared {
                return Err(ArchiveError::SizeMismatch { index });
            }
            let chunk = buffer.get(..n).ok_or(ArchiveError::Malformed)?;
            sink(chunk).map_err(|e| ArchiveError::Io(e.kind()))?;
        }
        if total != declared {
            return Err(ArchiveError::SizeMismatch { index });
        }
        Ok(total)
    }

    /// Extracts every entry under `dest`.
    ///
    /// `dest` must not exist, or be an empty directory. Directories and files are created with
    /// `create_new` (never following or creating a symbolic link), after every joined path has been
    /// checked to stay under the canonical `dest`; the mode is 0644 or 0755 (never a set-id bit). The
    /// bytes written are counted against the entry and total caps. On any error everything this call
    /// created is removed again.
    pub fn extract_to(&self, dest: &Path) -> Result<ExtractStats, ArchiveError> {
        let mut created = Created::default();
        match self.extract_inner(dest, &mut created) {
            Ok(stats) => Ok(stats),
            Err(e) => {
                created.rollback();
                Err(e)
            }
        }
    }

    fn extract_inner(
        &self,
        dest: &Path,
        created: &mut Created,
    ) -> Result<ExtractStats, ArchiveError> {
        match fs::symlink_metadata(dest) {
            Ok(meta) => {
                if !meta.is_dir() {
                    return Err(ArchiveError::DestinationNotEmpty);
                }
                let mut listing = fs::read_dir(dest).map_err(|e| ArchiveError::Io(e.kind()))?;
                if listing.next().is_some() {
                    return Err(ArchiveError::DestinationNotEmpty);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                make_dir(dest, created, true)?;
            }
            Err(e) => return Err(ArchiveError::Io(e.kind())),
        }
        let root = fs::canonicalize(dest).map_err(|e| ArchiveError::Io(e.kind()))?;
        let mut stats = ExtractStats::default();
        let mut total: u64 = 0;
        for (index, entry) in self.entries.iter().enumerate() {
            let relative = entry.name.strip_suffix('/').unwrap_or(&entry.name);
            let target = join_validated(&root, relative)?;
            if entry.is_dir {
                ensure_dir(&root, &target, created, &mut stats)?;
                continue;
            }
            let parent = target.parent().ok_or(ArchiveError::PathEscape)?;
            ensure_dir(&root, parent, created, &mut stats)?;
            let canonical_parent =
                fs::canonicalize(parent).map_err(|e| ArchiveError::Io(e.kind()))?;
            if !canonical_parent.starts_with(&root) {
                return Err(ArchiveError::PathEscape);
            }
            let mut file = create_file(&target, entry.executable)?;
            created.files.push(target.clone());
            let remaining = self.limits.max_total_bytes.saturating_sub(total);
            let cap = self.limits.max_entry_bytes.min(remaining);
            let written = {
                let mut sink = |chunk: &[u8]| io::Write::write_all(&mut file, chunk);
                self.stream_entry(index, cap, &mut sink)
                    .map_err(|e| match e {
                        ArchiveError::EntryTooLarge { .. } if cap < self.limits.max_entry_bytes => {
                            ArchiveError::TotalTooLarge {
                                limit: self.limits.max_total_bytes,
                            }
                        }
                        other => other,
                    })?
            };
            io::Write::flush(&mut file).map_err(|e| ArchiveError::Io(e.kind()))?;
            total = total.saturating_add(written);
            stats.files = stats.files.saturating_add(1);
            stats.bytes = stats.bytes.saturating_add(written);
        }
        Ok(stats)
    }
}

fn ratio_exceeded(size: u64, compressed: u64, limits: &Limits) -> bool {
    size > limits.ratio_min_entry_bytes
        && size > compressed.saturating_mul(limits.max_compression_ratio)
}

#[derive(Default)]
struct Created {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Created {
    fn rollback(&mut self) {
        for file in self.files.drain(..).rev() {
            let _ = fs::remove_file(file);
        }
        for dir in self.dirs.drain(..).rev() {
            let _ = fs::remove_dir(dir);
        }
    }
}

fn join_validated(root: &Path, relative: &str) -> Result<PathBuf, ArchiveError> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(ArchiveError::PathEscape);
        }
        path.push(component);
    }
    Ok(path)
}

fn make_dir(path: &Path, created: &mut Created, create_parents: bool) -> Result<(), ArchiveError> {
    if create_parents
        && let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        make_dir(parent, created, true)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {
            created.dirs.push(path.to_path_buf());
            set_mode(path, 0o755)?;
            Ok(())
        }
        Err(e) => Err(ArchiveError::Io(e.kind())),
    }
}

/// Creates `dir` and its missing parents below `root`, one level at a time.
fn ensure_dir(
    root: &Path,
    dir: &Path,
    created: &mut Created,
    stats: &mut ExtractStats,
) -> Result<(), ArchiveError> {
    let relative = dir
        .strip_prefix(root)
        .map_err(|_| ArchiveError::PathEscape)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(ArchiveError::PathEscape),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                make_dir(&current, created, false)?;
                stats.directories = stats.directories.saturating_add(1);
            }
            Err(e) => return Err(ArchiveError::Io(e.kind())),
        }
        let canonical = fs::canonicalize(&current).map_err(|e| ArchiveError::Io(e.kind()))?;
        if !canonical.starts_with(root) {
            return Err(ArchiveError::PathEscape);
        }
    }
    Ok(())
}

fn create_file(path: &Path, executable: bool) -> Result<fs::File, ArchiveError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o755 } else { 0o644 });
    }
    let file = options.open(path).map_err(|e| ArchiveError::Io(e.kind()))?;
    set_mode(path, if executable { 0o755 } else { 0o644 })?;
    Ok(file)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), ArchiveError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| ArchiveError::Io(e.kind()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), ArchiveError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Result<bool, NameProblem> {
        validate_entry_name(s.as_bytes(), &Limits::default())
    }

    #[test]
    fn plain_names_are_accepted() {
        assert_eq!(name("manifest.json"), Ok(false));
        assert_eq!(name("a/b/c.js"), Ok(false));
        assert_eq!(name("_locales/en/messages.json"), Ok(false));
        assert_eq!(name("dir/"), Ok(true));
        assert_eq!(name("a/b/"), Ok(true));
        assert_eq!(name("naïve/файл.txt"), Ok(false));
        assert_eq!(name("..hidden"), Ok(false));
        assert_eq!(name("a/...b"), Ok(false));
    }

    #[test]
    fn refused_names_name_their_problem() {
        assert_eq!(name(""), Err(NameProblem::Empty));
        assert_eq!(name("/"), Err(NameProblem::Absolute));
        assert_eq!(name("/etc/passwd"), Err(NameProblem::Absolute));
        assert_eq!(name("../evil"), Err(NameProblem::ParentComponent));
        assert_eq!(name("a/../../b"), Err(NameProblem::ParentComponent));
        assert_eq!(name("a/.."), Err(NameProblem::ParentComponent));
        assert_eq!(name("./a"), Err(NameProblem::CurrentComponent));
        assert_eq!(name("a/./b"), Err(NameProblem::CurrentComponent));
        assert_eq!(name("a//b"), Err(NameProblem::EmptyComponent));
        assert_eq!(name("//a"), Err(NameProblem::Absolute));
        assert_eq!(name("a//"), Err(NameProblem::EmptyComponent));
        assert_eq!(name("C:\\x"), Err(NameProblem::Backslash));
        assert_eq!(name("C:/x"), Err(NameProblem::DriveLetter));
        assert_eq!(name("c:x"), Err(NameProblem::DriveLetter));
        assert_eq!(name("a\\..\\b"), Err(NameProblem::Backslash));
        assert_eq!(name("a\0b"), Err(NameProblem::Nul));
        assert_eq!(name("a\nb"), Err(NameProblem::ControlCharacter));
        assert_eq!(name("a\u{7f}b"), Err(NameProblem::ControlCharacter));
        assert_eq!(name("a\u{85}b"), Err(NameProblem::ControlCharacter));
        assert_eq!(name("a\u{202e}b"), Err(NameProblem::ControlCharacter));
        assert_eq!(
            validate_entry_name(&[b'a', 0xff, b'b'], &Limits::default()),
            Err(NameProblem::NotUtf8)
        );
    }

    #[test]
    fn name_length_edge() {
        let limits = Limits::default();
        let at = "a".repeat(limits.max_path_bytes);
        assert_eq!(validate_entry_name(at.as_bytes(), &limits), Ok(false));
        let over = "a".repeat(limits.max_path_bytes + 1);
        assert_eq!(
            validate_entry_name(over.as_bytes(), &limits),
            Err(NameProblem::TooLong)
        );
    }

    #[test]
    fn name_depth_edge() {
        let limits = Limits::default();
        let at = vec!["d"; limits.max_path_depth].join("/");
        assert_eq!(validate_entry_name(at.as_bytes(), &limits), Ok(false));
        let over = vec!["d"; limits.max_path_depth + 1].join("/");
        assert_eq!(
            validate_entry_name(over.as_bytes(), &limits),
            Err(NameProblem::TooDeep)
        );
    }

    #[test]
    fn ratio_rule_applies_above_the_minimum_size_only() {
        let limits = Limits::default();
        let mib = limits.ratio_min_entry_bytes;
        assert!(!ratio_exceeded(mib, 0, &limits));
        assert!(ratio_exceeded(mib + 1, 0, &limits));
        assert!(!ratio_exceeded(mib + 1, (mib + 1).div_ceil(200), &limits));
        assert!(ratio_exceeded(
            200 * 6000 + 1,
            5999,
            &Limits {
                ratio_min_entry_bytes: 0,
                ..Limits::default()
            }
        ));
    }

    #[test]
    fn default_limits_are_the_documented_numbers() {
        let l = Limits::default();
        assert_eq!(l.max_package_bytes, 209_715_200);
        assert_eq!(l.max_entries, 20_000);
        assert_eq!(l.max_entry_bytes, 104_857_600);
        assert_eq!(l.max_total_bytes, 524_288_000);
        assert_eq!(l.max_compression_ratio, 200);
        assert_eq!(l.ratio_min_entry_bytes, 1_048_576);
        assert_eq!(l.max_path_bytes, 512);
        assert_eq!(l.max_path_depth, 32);
        assert_eq!(l.max_manifest_bytes, 1_048_576);
        assert_eq!(l.max_json_depth, 64);
    }
}
