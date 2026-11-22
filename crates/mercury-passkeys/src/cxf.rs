// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Import and export of passkeys in the FIDO Credential Exchange Format (CXF 1.0), with the
//! `credential-exchange-format` crate's types (EC46).
//!
//! # What is exchanged
//!
//! Only passkeys. [`export_cxf`] writes one account with one item per passkey; [`import_cxf`]
//! accepts any valid CXF 1.0 file, takes its passkeys, and counts every other credential kind it
//! skips (a real export holds passwords, notes and cards; none of that is an error).
//!
//! # An export holds private keys in the clear
//!
//! CXF stores a passkey's private key as PKCS#8 DER, base64url encoded, with no encryption.
//! [`export_cxf`] therefore needs an [`ExportAcknowledgement`] (so the caller cannot export without
//! naming what it does, and the interface must have warned the user), returns the file in a
//! buffer that is wiped when dropped, and [`write_export_file`] creates the file readable by its
//! owner only and never replaces an existing file. Encrypting the export is not done here.
//!
//! # Rules a file must meet
//!
//! An import refuses a file as a whole, with [`PasskeyError::InvalidExchangeFile`] and a fixed
//! text, if it is larger than [`MAX_IMPORT_BYTES`], is not valid UTF-8 JSON of the CXF shape (or
//! has anything after the JSON value), has a major version other than 1, has more than
//! [`MAX_IMPORT_ITEMS`] items, holds a passkey entry that is malformed, whose private key is not
//! PKCS#8 DER, or an ES256 key that does not parse, or whose ids and names break the store's rules
//! ([`validate_record`]). There is never a partial import. A well-formed key of another algorithm,
//! or a passkey with extensions this crate cannot keep, is skipped and reported instead.
//!
//! # Memory
//!
//! Every buffer this module creates that holds a key is wiped when dropped, and the parsed file is
//! wiped of its passkey keys before it is dropped. The dependencies do not wipe: while a file is
//! parsed or written, copies of the encoded key may remain in freed memory inside `serde_json`,
//! `coset` and the CXF types, and the other credentials of a file (passwords, for one) are not
//! wiped at all. Callers should hold the file they read in a [`Zeroizing`] buffer (see
//! [`read_import_file`]).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

use credential_exchange_format::{
    Account, B64Url, Credential, CredentialScope, Header, Item, PasskeyCredential, Version,
};
use passkey::crypto::coset::{CborSerializable, CoseKey, RegisteredLabelWithPrivate, iana};
use passkey::crypto::rust_crypto::RustCryptoSecretKey;
use passkey::crypto::{Pkcs8, SecretKeyT};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    COSE_ES256, PasskeyError, PasskeyKey, PasskeyRecord, PasskeySummary, PrivateKey,
    validate_record,
};

/// The largest file [`import_cxf`] reads: 64 MiB.
pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
/// The most items (across all accounts) [`import_cxf`] accepts.
pub const MAX_IMPORT_ITEMS: usize = 100_000;
/// The relying party id this application puts in an export's header.
pub const EXPORTER_RP_ID: &str = "io.github.thepigdestroyer.mercury-browser";
/// The name this application puts in an export's header.
pub const EXPORTER_DISPLAY_NAME: &str = "Mercury Browser";

/// The most credentials (across all items) [`import_cxf`] accepts.
const MAX_IMPORT_CREDENTIALS: usize = 1_000_000;
/// The most distinct credential kinds [`Imported::skipped_kinds`] names; the rest count as `other`.
const MAX_SKIPPED_KIND_NAMES: usize = 64;
/// The longest credential kind name kept, in characters.
const MAX_KIND_NAME_CHARS: usize = 64;
const ID_BYTES: usize = 16;

/// The caller's statement that it knows an export holds private keys in the clear. There is one
/// value, so a call site cannot export without naming it; the interface must have warned the user
/// before it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportAcknowledgement {
    /// The export file will contain every exported passkey's private key unencrypted.
    PrivateKeysInClear,
}

/// Why a passkey was not exported or not imported. Carries no name, id or key, so it is safe to
/// log and to count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// Export: the passkey has no user handle, and CXF requires one.
    NotDiscoverable,
    /// Export: the passkey has a non-zero signature counter, which the CXF specification says an
    /// exporter must leave out (an importer sets zero and never counts).
    NonZeroCounter,
    /// The key is not an ES256 key; this crate keeps ES256 keys only.
    UnsupportedAlgorithm,
    /// Export: the stored key could not be read.
    UnreadableKey,
    /// Export: the stored passkey breaks the rules an import would apply to it, so the file would
    /// not import again.
    InvalidRecord,
    /// Import: the passkey carries an extension (hmac-secret or PRF, credential blob, large blob)
    /// that this crate cannot keep. Importing the key without it would silently change what the
    /// passkey does for the site.
    UnsupportedExtension,
}

/// A CXF file made by [`export_cxf`]: the JSON bytes, which hold private keys in the clear, and
/// what was left out. The bytes are wiped when this value is dropped.
///
/// `Debug` shows counts only.
pub struct Exported {
    bytes: Zeroizing<Vec<u8>>,
    /// How many passkeys the file holds.
    pub exported: usize,
    /// One entry for every passkey that was left out, in input order. The CXF specification says
    /// an exporter informs the user of these.
    pub skipped: Vec<SkipReason>,
}

impl Exported {
    /// The file contents (UTF-8 JSON). Holds private keys in the clear.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for Exported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Exported")
            .field("bytes", &format_args!("<{} bytes>", self.bytes.len()))
            .field("exported", &self.exported)
            .field("skipped", &self.skipped.len())
            .finish()
    }
}

/// What [`import_cxf`] found in a file that was accepted as a whole.
///
/// `Debug` shows counts only.
pub struct Imported {
    /// The passkeys to save, in file order, each with a signature counter of `None` and its private
    /// key as COSE_Key. Duplicates inside the file (same relying party and credential id, same
    /// key) appear once.
    pub records: Vec<PasskeyRecord>,
    /// One entry for every passkey that was not imported.
    pub skipped: Vec<SkipReason>,
    /// Credentials that are not passkeys and were skipped, counted by CXF type name
    /// (`basic-auth`, `note`, ...). A name the file invented is cut to 64 characters, and beyond 64
    /// distinct names the count goes under `other`.
    pub skipped_kinds: BTreeMap<String, usize>,
}

impl fmt::Debug for Imported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Imported")
            .field("records", &self.records.len())
            .field("skipped", &self.skipped.len())
            .field("skipped_kinds", &self.skipped_kinds.values().sum::<usize>())
            .finish()
    }
}

/// Writes `records` as a CXF 1.0 file: one account, one item per passkey that can be exported.
///
/// `now_secs` is the time in seconds since the Unix epoch for the header, and for items whose
/// passkey has no dates. A passkey that cannot be exported is left out and listed in
/// [`Exported::skipped`] (see [`SkipReason`]); the rest are exported. The file holds every
/// exported private key in the clear (see the module documentation).
///
/// # Errors
///
/// [`PasskeyError::RandomUnavailable`] if the operating system's random source fails (ids are
/// random), and [`PasskeyError::InvalidInput`] if there are more passkeys than an import accepts or
/// the file would be larger than an import accepts.
pub fn export_cxf(
    records: &[PasskeyRecord],
    ack: ExportAcknowledgement,
    now_secs: u64,
) -> Result<Exported, PasskeyError> {
    let ExportAcknowledgement::PrivateKeysInClear = ack;
    if records.len() > MAX_IMPORT_ITEMS {
        return Err(PasskeyError::InvalidInput(
            "there are more passkeys than one exchange file can hold",
        ));
    }
    let mut items = Vec::with_capacity(records.len());
    let mut skipped = Vec::new();
    for record in records {
        match export_item(record, now_secs)? {
            Converted::Done(item) => items.push(*item),
            Converted::Skip(reason) => skipped.push(reason),
        }
    }
    let exported = items.len();
    let header = HeaderGuard(Header {
        version: Version { major: 1, minor: 0 },
        exporter_rp_id: EXPORTER_RP_ID.to_owned(),
        exporter_display_name: EXPORTER_DISPLAY_NAME.to_owned(),
        timestamp: now_secs,
        accounts: vec![Account {
            id: random_id()?,
            username: String::new(),
            email: String::new(),
            full_name: None,
            collections: Vec::new(),
            items,
            extensions: None,
        }],
    });
    let mut writer = WipingWriter::default();
    serde_json::to_writer(&mut writer, &header.0).map_err(|_| {
        PasskeyError::InvalidInput("the export would be larger than an import accepts")
    })?;
    Ok(Exported {
        bytes: writer.buffer,
        exported,
        skipped,
    })
}

/// Reads a CXF file and returns its passkeys. See the module documentation for what is refused;
/// a refused file imports nothing.
///
/// The passkeys are returned, not saved: the caller saves them through
/// [`PasskeyStore::save`](crate::PasskeyStore::save), and the record's dates are the file's
/// (seconds, as milliseconds).
///
/// # Errors
///
/// [`PasskeyError::InvalidExchangeFile`], with a fixed text that says what is wrong with the file
/// and never quotes it.
pub fn import_cxf(bytes: &[u8]) -> Result<Imported, PasskeyError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(refuse("the file is too large"));
    }
    if bytes.is_empty() {
        return Err(refuse("the file is empty"));
    }
    let header = HeaderGuard(serde_json::from_slice::<Header>(bytes).map_err(|error| {
        // Never `error.to_string()`: serde_json can quote the value it choked on, which may be a key.
        refuse(match error.classify() {
            serde_json::error::Category::Eof => "the file ends early",
            serde_json::error::Category::Syntax | serde_json::error::Category::Io => {
                "the file is not valid JSON, or has data after the JSON value"
            }
            serde_json::error::Category::Data => {
                "the file does not have the structure of a credential exchange file"
            }
        })
    })?);
    if header.0.version.major != 1 {
        return Err(refuse("the format version is not supported"));
    }
    let items = header.0.accounts.iter().try_fold(0usize, |total, account| {
        total.checked_add(account.items.len())
    });
    if items.is_none_or(|total| total > MAX_IMPORT_ITEMS)
        || header.0.accounts.len() > MAX_IMPORT_ITEMS
    {
        return Err(refuse("the file holds too many items"));
    }

    let mut records: Vec<PasskeyRecord> = Vec::new();
    let mut skipped = Vec::new();
    let mut skipped_kinds = BTreeMap::new();
    let mut credentials = 0usize;
    for item in header.0.accounts.iter().flat_map(|account| &account.items) {
        for credential in &item.credentials {
            credentials += 1;
            if credentials > MAX_IMPORT_CREDENTIALS {
                return Err(refuse("the file holds too many credentials"));
            }
            match credential {
                Credential::Passkey(passkey) => match import_passkey(item, passkey)? {
                    Converted::Done(record) => {
                        if !is_duplicate(&records, &record)? {
                            records.push(*record);
                        }
                    }
                    Converted::Skip(reason) => skipped.push(reason),
                },
                // An entry that says it is a passkey but did not parse as one falls through to
                // `Unknown`; it must not pass as "some other kind".
                Credential::Unknown { ty, .. } if ty == "passkey" => {
                    return Err(refuse("a passkey entry is malformed"));
                }
                other => count_kind(&mut skipped_kinds, other),
            }
        }
    }
    Ok(Imported {
        records,
        skipped,
        skipped_kinds,
    })
}

/// Reads a file for [`import_cxf`]: refuses a file larger than [`MAX_IMPORT_BYTES`] without reading
/// it, and returns the contents in a buffer that is wiped when dropped.
///
/// # Errors
///
/// [`PasskeyError::InvalidExchangeFile`] if the file is too large, [`PasskeyError::Os`] if it
/// cannot be read.
pub fn read_import_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, PasskeyError> {
    let file = std::fs::File::open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if metadata.len() > MAX_IMPORT_BYTES as u64 {
        return Err(refuse("the file is too large"));
    }
    // The file may grow after the length check: read at most one byte more than the limit.
    let mut contents = Zeroizing::new(Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(0)
            .saturating_add(1),
    ));
    file.take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut contents)
        .map_err(io_error)?;
    if contents.len() > MAX_IMPORT_BYTES {
        return Err(refuse("the file is too large"));
    }
    Ok(contents)
}

/// Writes an export to a new file that only its owner can read (mode 0600) and never replaces an
/// existing file or follows a symbolic link at `path`. The data is flushed to disk before the call
/// returns; if writing fails, the partial file is removed.
///
/// # Errors
///
/// [`PasskeyError::InvalidInput`] if `path` already exists, [`PasskeyError::Os`] for other file
/// system errors, and [`PasskeyError::Unsupported`] on a platform without file modes.
pub fn write_export_file(path: &Path, exported: &Exported) -> Result<(), PasskeyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    PasskeyError::InvalidInput("the export file already exists")
                } else {
                    io_error(error)
                }
            })?;
        let written = file
            .write_all(exported.as_bytes())
            .and_then(|()| file.sync_all());
        if let Err(error) = written {
            drop(file);
            // Best effort: the partial file holds private keys.
            let _ = std::fs::remove_file(path);
            return Err(io_error(error));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, exported, OpenOptions::new());
        Err(PasskeyError::Unsupported)
    }
}

// ---- export ----------------------------------------------------------------------------------

enum Converted<T> {
    Done(Box<T>),
    Skip(SkipReason),
}

fn export_item(record: &PasskeyRecord, now_secs: u64) -> Result<Converted<Item>, PasskeyError> {
    let summary = &record.summary;
    if validate_record(record).is_err() {
        return Ok(Converted::Skip(SkipReason::InvalidRecord));
    }
    let Some(user_handle) = &summary.user_handle else {
        return Ok(Converted::Skip(SkipReason::NotDiscoverable));
    };
    if summary.counter.is_some_and(|counter| counter != 0) {
        return Ok(Converted::Skip(SkipReason::NonZeroCounter));
    }
    if summary.algorithm != COSE_ES256 {
        return Ok(Converted::Skip(SkipReason::UnsupportedAlgorithm));
    }
    let key = match pkcs8_from_private_key(&record.private_key) {
        Ok(key) => key,
        Err(reason) => return Ok(Converted::Skip(reason)),
    };
    let created = summary
        .created_at_ms
        .and_then(ms_to_secs)
        .unwrap_or(now_secs);
    let modified = summary
        .modified_at_ms
        .and_then(ms_to_secs)
        .unwrap_or(created);
    let rp_id = &summary.key.rp_id;
    Ok(Converted::Done(Box::new(Item {
        id: random_id()?,
        creation_at: Some(created),
        modified_at: Some(modified),
        title: rp_id.clone(),
        subtitle: Some(summary.username.clone()),
        favorite: None,
        scope: Some(CredentialScope {
            urls: vec![format!("https://{rp_id}")],
            android_apps: Vec::new(),
        }),
        credentials: vec![Credential::Passkey(Box::new(PasskeyCredential {
            credential_id: B64Url::from(summary.key.credential_id.clone()),
            rp_id: rp_id.clone(),
            username: summary.username.clone(),
            user_display_name: summary.display_name.clone(),
            user_handle: B64Url::from(user_handle.clone()),
            key: B64Url::from(key.to_vec()),
            fido2_extensions: None,
        }))],
        tags: None,
        extensions: None,
    })))
}

/// The PKCS#8 DER of the ES256 key in a COSE_Key encoding.
fn pkcs8_from_private_key(key: &PrivateKey) -> Result<Zeroizing<Vec<u8>>, SkipReason> {
    let mut cose = CoseKey::from_slice(key.expose()).map_err(|_| SkipReason::UnreadableKey)?;
    let converted = (|| {
        if cose.alg != Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::ES256)) {
            return Err(SkipReason::UnsupportedAlgorithm);
        }
        let secret =
            RustCryptoSecretKey::from_cose_key(&cose).map_err(|_| SkipReason::UnreadableKey)?;
        secret
            .to_pkcs8()
            .map(Zeroizing::new)
            .map_err(|_| SkipReason::UnreadableKey)
    })();
    wipe_cose_key(&mut cose);
    converted
}

fn wipe_cose_key(key: &mut CoseKey) {
    use passkey::crypto::coset::cbor::Value;
    for (_, value) in &mut key.params {
        if let Value::Bytes(bytes) = value {
            bytes.zeroize();
        }
    }
}

fn ms_to_secs(ms: i64) -> Option<u64> {
    u64::try_from(ms).ok().map(|ms| ms / 1000)
}

fn random_id() -> Result<B64Url, PasskeyError> {
    let mut id = vec![0u8; ID_BYTES];
    getrandom::fill(&mut id).map_err(|_| PasskeyError::RandomUnavailable)?;
    Ok(B64Url::from(id))
}

/// A growing buffer that wipes the old allocation each time it grows, so no partial copy of the
/// export is left behind, and stops at the size an import accepts.
#[derive(Default)]
struct WipingWriter {
    buffer: Zeroizing<Vec<u8>>,
}

impl Write for WipingWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let needed = self.buffer.len() + data.len();
        if needed > MAX_IMPORT_BYTES {
            return Err(io::Error::other("the export is too large"));
        }
        if needed > self.buffer.capacity() {
            let capacity = needed
                .max(self.buffer.capacity().saturating_mul(2))
                .max(4096);
            let mut grown = Zeroizing::new(Vec::with_capacity(capacity.min(MAX_IMPORT_BYTES)));
            grown.extend_from_slice(&self.buffer);
            self.buffer = grown;
        }
        self.buffer.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ---- import ----------------------------------------------------------------------------------

fn refuse(reason: &'static str) -> PasskeyError {
    PasskeyError::InvalidExchangeFile(reason)
}

fn import_passkey(
    item: &Item,
    passkey: &PasskeyCredential,
) -> Result<Converted<PasskeyRecord>, PasskeyError> {
    let (private_key, supported) = match classify_pkcs8(passkey.key.as_ref())? {
        KeyKind::Es256 => (private_key_from_pkcs8(passkey.key.as_ref())?, true),
        // A stand-in so the ids and names are validated even when the key is skipped.
        KeyKind::Other => (PrivateKey::new(vec![0]), false),
    };
    let ms = |secs: Option<u64>| {
        secs.and_then(|secs| secs.checked_mul(1000))
            .and_then(|ms| i64::try_from(ms).ok())
    };
    let record = PasskeyRecord {
        summary: PasskeySummary {
            key: PasskeyKey {
                rp_id: passkey.rp_id.clone(),
                credential_id: passkey.credential_id.as_ref().to_vec(),
            },
            user_handle: Some(passkey.user_handle.as_ref().to_vec()),
            username: passkey.username.clone(),
            display_name: passkey.user_display_name.clone(),
            algorithm: COSE_ES256,
            counter: None,
            created_at_ms: ms(item.creation_at),
            modified_at_ms: ms(item.modified_at),
        },
        private_key,
    };
    validate_record(&record).map_err(|_| {
        refuse("a passkey has an id, user handle, name or key that this application cannot store")
    })?;
    if !supported {
        return Ok(Converted::Skip(SkipReason::UnsupportedAlgorithm));
    }
    if let Some(extensions) = &passkey.fido2_extensions
        && (extensions.hmac_credentials.is_some()
            || extensions.cred_blob.is_some()
            || extensions.large_blob.is_some())
    {
        return Ok(Converted::Skip(SkipReason::UnsupportedExtension));
    }
    Ok(Converted::Done(Box::new(record)))
}

/// Whether `record` is already in `records`; an entry with the same passkey but another key is a
/// conflict the file cannot be trusted over.
fn is_duplicate(records: &[PasskeyRecord], record: &PasskeyRecord) -> Result<bool, PasskeyError> {
    match records
        .iter()
        .find(|existing| existing.summary.key == record.summary.key)
    {
        None => Ok(false),
        Some(existing) if existing.private_key.same_as(&record.private_key) => Ok(true),
        Some(_) => Err(refuse(
            "the file lists one passkey twice with different keys",
        )),
    }
}

fn count_kind(kinds: &mut BTreeMap<String, usize>, credential: &Credential) {
    let name = match credential {
        Credential::Address(_) => "address",
        Credential::ApiKey(_) => "api-key",
        Credential::BasicAuth(_) => "basic-auth",
        Credential::CreditCard(_) => "credit-card",
        Credential::CustomFields(_) => "custom-fields",
        Credential::DriversLicense(_) => "drivers-license",
        Credential::File(_) => "file",
        Credential::GeneratedPassword(_) => "generated-password",
        Credential::IdentityDocument(_) => "identity-document",
        Credential::ItemReference(_) => "item-reference",
        Credential::Note(_) => "note",
        Credential::Passkey(_) => "passkey",
        Credential::Passport(_) => "passport",
        Credential::PersonName(_) => "person-name",
        Credential::SshKey(_) => "ssh-key",
        Credential::Totp(_) => "totp",
        Credential::Wifi(_) => "wifi",
        Credential::Unknown { ty, .. } => {
            let name: String = ty
                .chars()
                .take(MAX_KIND_NAME_CHARS)
                .map(|c| if c.is_control() { '?' } else { c })
                .collect();
            return count_name(kinds, name);
        }
        _ => "other",
    };
    count_name(kinds, name.to_owned());
}

fn count_name(kinds: &mut BTreeMap<String, usize>, name: String) {
    let name = if !kinds.contains_key(&name) && kinds.len() >= MAX_SKIPPED_KIND_NAMES {
        "other".to_owned()
    } else {
        name
    };
    *kinds.entry(name).or_insert(0) += 1;
}

enum KeyKind {
    Es256,
    Other,
}

const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

/// Reads the algorithm out of a PKCS#8 `PrivateKeyInfo`. Refuses what is not one; a well-formed
/// key that is not on P-256 is `Other`.
fn classify_pkcs8(der: &[u8]) -> Result<KeyKind, PasskeyError> {
    let Some((algorithm, parameters)) = pkcs8_algorithm(der) else {
        return Err(refuse("a passkey's private key is not PKCS#8 DER"));
    };
    Ok(
        if algorithm == OID_EC_PUBLIC_KEY && parameters == Some(OID_PRIME256V1) {
            KeyKind::Es256
        } else {
            KeyKind::Other
        },
    )
}

/// The algorithm OID of a PKCS#8 `PrivateKeyInfo` (RFC 5958) and, if its parameters are an OID,
/// that OID. `None` if the bytes are not a well-formed `PrivateKeyInfo` header.
fn pkcs8_algorithm(der: &[u8]) -> Option<(&[u8], Option<&[u8]>)> {
    const SEQUENCE: u8 = 0x30;
    const INTEGER: u8 = 0x02;
    const OCTET_STRING: u8 = 0x04;
    const OID: u8 = 0x06;

    let (tag, body, rest) = der_read(der)?;
    if tag != SEQUENCE || !rest.is_empty() {
        return None;
    }
    let (tag, version, body) = der_read(body)?;
    if tag != INTEGER || !matches!(version, [0] | [1]) {
        return None;
    }
    let (tag, algorithm, body) = der_read(body)?;
    if tag != SEQUENCE {
        return None;
    }
    let (tag, key, _attributes) = der_read(body)?;
    if tag != OCTET_STRING || key.is_empty() {
        return None;
    }
    let (tag, oid, parameters) = der_read(algorithm)?;
    if tag != OID {
        return None;
    }
    let parameter_oid = if parameters.is_empty() {
        None
    } else {
        let (tag, parameter, tail) = der_read(parameters)?;
        if !tail.is_empty() {
            return None;
        }
        (tag == OID).then_some(parameter)
    };
    Some((oid, parameter_oid))
}

/// One DER element: its tag, its content, and what follows it. Definite lengths only.
fn der_read(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let (length_bytes, rest) = rest.split_at(count);
        let length = length_bytes
            .iter()
            .fold(0usize, |length, &byte| (length << 8) | usize::from(byte));
        (length, rest)
    };
    if rest.len() < length {
        return None;
    }
    let (content, rest) = rest.split_at(length);
    Some((tag, content, rest))
}

/// The COSE_Key encoding of an ES256 key given as PKCS#8 DER.
fn private_key_from_pkcs8(der: &[u8]) -> Result<PrivateKey, PasskeyError> {
    let unreadable = || refuse("a passkey's ES256 private key could not be read");
    let secret = RustCryptoSecretKey::from_pkcs8(der).map_err(|_| unreadable())?;
    let mut cose = secret.to_cose_key();
    let encoded = cose.clone().to_vec();
    wipe_cose_key(&mut cose);
    encoded.map(PrivateKey::new).map_err(|_| unreadable())
}

// ---- wiping the parsed file ------------------------------------------------------------------

/// A parsed or built file whose passkey keys are wiped when it is dropped, on every path.
struct HeaderGuard(Header);

impl Drop for HeaderGuard {
    fn drop(&mut self) {
        for account in &mut self.0.accounts {
            for item in &mut account.items {
                for credential in &mut item.credentials {
                    match credential {
                        Credential::Passkey(passkey) => {
                            let key = std::mem::replace(&mut passkey.key, B64Url::from(Vec::new()));
                            drop(Zeroizing::new(Vec::<u8>::from(key)));
                        }
                        Credential::Unknown { content, .. } => {
                            content.values_mut().for_each(wipe_json);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn wipe_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(string) => string.zeroize(),
        serde_json::Value::Array(values) => values.iter_mut().for_each(wipe_json),
        serde_json::Value::Object(map) => map.values_mut().for_each(wipe_json),
        _ => {}
    }
}

fn io_error(error: io::Error) -> PasskeyError {
    match error.raw_os_error() {
        Some(code) => PasskeyError::Os {
            code,
            message: io::Error::from_raw_os_error(code).to_string(),
        },
        None => PasskeyError::Internal("a file operation failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if content.len() < 0x80 {
            out.push(content.len() as u8);
        } else {
            out.extend([0x82, (content.len() >> 8) as u8, content.len() as u8]);
        }
        out.extend(content);
        out
    }

    fn pkcs8(oid: &[u8], parameters: Option<Vec<u8>>) -> Vec<u8> {
        let mut algorithm = tlv(0x06, oid);
        algorithm.extend(parameters.unwrap_or_default());
        let mut body = tlv(0x02, &[0]);
        body.extend(tlv(0x30, &algorithm));
        body.extend(tlv(0x04, &[1, 2, 3]));
        tlv(0x30, &body)
    }

    #[test]
    fn reads_the_algorithm_and_curve() {
        let der = pkcs8(OID_EC_PUBLIC_KEY, Some(tlv(0x06, OID_PRIME256V1)));
        assert_eq!(
            pkcs8_algorithm(&der),
            Some((OID_EC_PUBLIC_KEY, Some(OID_PRIME256V1)))
        );
        let rsa = pkcs8(
            &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01],
            Some(tlv(0x05, &[])),
        );
        assert_eq!(
            pkcs8_algorithm(&rsa).map(|(_, parameters)| parameters),
            Some(None)
        );
    }

    #[test]
    fn rejects_malformed_der() {
        let good = pkcs8(OID_EC_PUBLIC_KEY, Some(tlv(0x06, OID_PRIME256V1)));
        for length in 0..good.len() {
            assert!(pkcs8_algorithm(&good[..length]).is_none(), "{length}");
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(pkcs8_algorithm(&trailing).is_none());
        // Indefinite length, a length longer than the data, and a four-byte length of 4 GiB.
        assert!(der_read(&[0x30, 0x80, 0, 0]).is_none());
        assert!(der_read(&[0x04, 0x05, 1]).is_none());
        assert!(der_read(&[0x04, 0x84, 0xff, 0xff, 0xff, 0xff, 1]).is_none());
        assert!(der_read(&[0x04, 0x85, 0, 0, 0, 0, 0]).is_none());
        // Wrong version.
        let mut bad_version = good;
        bad_version[4] = 7;
        assert!(pkcs8_algorithm(&bad_version).is_none());
    }

    #[test]
    fn kind_names_are_bounded() {
        let mut kinds = BTreeMap::new();
        for index in 0..(MAX_SKIPPED_KIND_NAMES + 10) {
            count_name(&mut kinds, format!("kind-{index}"));
        }
        assert_eq!(kinds.len(), MAX_SKIPPED_KIND_NAMES + 1);
        assert_eq!(kinds.get("other"), Some(&10));
        count_name(&mut kinds, "kind-0".to_owned());
        assert_eq!(kinds.get("kind-0"), Some(&2));
    }

    #[test]
    fn wiping_writer_keeps_the_data_and_the_limit() {
        let mut writer = WipingWriter::default();
        for _ in 0..1000 {
            writer.write_all(&[7u8; 100]).unwrap();
        }
        assert_eq!(writer.buffer.len(), 100_000);
        assert!(writer.buffer.iter().all(|&byte| byte == 7));
        let mut full = WipingWriter::default();
        assert!(full.write(&vec![0; MAX_IMPORT_BYTES]).is_ok());
        assert!(full.write(&[0]).is_err());
    }
}
