// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Passkeys in a macOS keychain through the Security framework's `SecItem` API.
//!
//! The structure follows `mercury-passwords`' Keychain backend (a later clean-up can share the
//! two). Every call names its keychain: items are added with `kSecUseKeychain` and searched,
//! updated and deleted with `kSecMatchSearchList` set to that one keychain. There is no code path
//! that leaves the keychain unspecified; [`KeychainPasskeyBackend::login`] resolves the user's
//! default keychain (`SecKeychainCopyDefault`) and then passes it explicitly like any other.
//!
//! Item layout (one generic-password item per passkey):
//! - service (`svce`): [`KEYCHAIN_SERVICE`] for every passkey;
//! - account (`acct`): `v1|<rp id>|<credential id, base64url without padding>`;
//! - label (`labl`): `<rp id> (<user name>)`, or the relying party id alone, so the item is
//!   recognisable in Keychain Access;
//! - description (`desc`, shown as "Kind"): `Mercury Browser passkey`;
//! - comment (`icmt`): a small JSON object with everything that is not secret: the format version
//!   `v`, the user handle `uh` (base64url or `null`), the user name `un`, the display name `dn`,
//!   the signature counter `c` (a number or `null`) and the COSE algorithm `a`;
//! - data: the private COSE_Key (CBOR), the only secret;
//! - creation and modification dates (`cdat`, `mdat`): kept by the Keychain itself.
//!
//! Listing reads attributes only, never secret data. With a locked keychain, `SecItemCopyMatching`
//! still answers an attributes-only query (the attributes of a file keychain are not encrypted
//! with the user's secret), so listing and per-site deletion lists work while reading a private
//! key fails with [`PasskeyError::Locked`].
//!
//! Access control is explicit (see [`AccessPolicy`]): every item is added with a `SecAccess`
//! naming the trusted applications, instead of leaving the choice to the system.
//!
//! The private key crosses the Security framework API in a `CFData` the backend makes itself
//! and wipes after the call. The copy `SecItemCopyMatching` makes when a key is read lives in
//! Core Foundation memory that this API cannot wipe; the copy handed to the caller is a
//! [`PrivateKey`], which is wiped on drop.

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::ptr;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType, kCFAllocatorDefault};
use core_foundation::data::{
    CFData, CFDataAppendBytes, CFDataCreateMutable, CFDataGetMutableBytePtr, CFDataRef,
};
use core_foundation::dictionary::{CFDictionaryRef, CFMutableDictionary};
use core_foundation::string::{CFString, CFStringRef};
use data_encoding::BASE64URL_NOPAD;
use security_framework::base::Error as SecError;
use security_framework::item::{
    ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, ItemUpdateOptions, ItemUpdateValue,
    Limit, Location, SearchResult, update_item,
};
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use serde_json::{Value, json};
use zeroize::Zeroize;

use crate::{
    AddOutcome, MAX_CREDENTIAL_ID_BYTES, PasskeyBackend, PasskeyError, PasskeyKey, PasskeyRecord,
    PasskeySummary, PrivateKey, SaveOutcome, validate_rp_id,
};

/// The Keychain service name of every passkey this application saves: the application's bundle
/// identifier followed by `.passkeys`.
pub const KEYCHAIN_SERVICE: &str = "io.github.thepigdestroyer.mercury-browser.passkeys";

/// The description (`desc`, "Kind" in Keychain Access) of every passkey item.
pub const ITEM_DESCRIPTION: &str = "Mercury Browser passkey";

/// Longest comment attribute the backend writes or accepts, in bytes.
const MAX_COMMENT_BYTES: usize = 16 * 1024;
/// The format version written to the comment and to the account.
const FORMAT_VERSION: u64 = 1;
/// Longest user name shown in an item's label, in characters.
const MAX_LABEL_NAME_CHARS: usize = 100;

// `OSStatus` codes from `Security/SecBase.h`.
const ERR_SEC_USER_CANCELED: i32 = -128;
const ERR_SEC_AUTH_FAILED: i32 = -25293;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

/// Who may read a passkey's private key without the system asking the user. It is the access list
/// every new item is created with, and a field of the backend, so a stricter policy later (a
/// Developer ID signature, the data-protection keychain) is a new value, not a new code path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessPolicy {
    /// Only this application (`SecTrustedApplicationCreateFromPath` with a null path, the running
    /// executable) is trusted; any other program makes the system ask the user. This is the
    /// access list a new item would get by default, written down explicitly.
    #[default]
    ThisApplicationOnly,
}

/// Passkeys in a macOS keychain, one generic-password item per passkey.
///
/// See the module documentation for the item layout. Every operation is restricted to the one
/// keychain this backend was made for. Listing reads attributes only and never secret data. Items
/// under the service whose account does not decode as a version-1 passkey account are skipped and
/// never removed. An item with a valid account but a missing or damaged comment is listed as
/// skipped, and `stored_keys` still returns its key so that `remove_all` removes it.
///
/// `Debug` shows only whether the backend uses the default keychain or a file, and the access
/// policy.
pub struct KeychainPasskeyBackend {
    target: Target,
    access_policy: AccessPolicy,
}

enum Target {
    /// The user's default keychain, looked up on every operation.
    Default,
    /// One keychain file.
    File(SecKeychain),
}

impl fmt::Debug for KeychainPasskeyBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeychainPasskeyBackend")
            .field(
                "keychain",
                &match self.target {
                    Target::Default => "default",
                    Target::File(_) => "file",
                },
            )
            .field("access_policy", &self.access_policy)
            .finish()
    }
}

impl KeychainPasskeyBackend {
    /// The user's default keychain. Used by the application only.
    pub fn login() -> Self {
        Self {
            target: Target::Default,
            access_policy: AccessPolicy::default(),
        }
    }

    /// A keychain file at `path`, created with `password` if absent. For tests and tools.
    ///
    /// An existing file is opened and unlocked with `password`; a wrong password gives
    /// [`PasskeyError::AccessDenied`]. A new file is created unlocked.
    ///
    /// `SecKeychainUnlock` checks the password only when the keychain is locked: on an unlocked
    /// keychain (a new one starts unlocked and macOS keeps it so until its idle timer, sleep or a
    /// lock request) it succeeds with any password. So an existing keychain is locked first, and
    /// only then unlocked with `password`; if it cannot be locked, `file` fails rather than trust
    /// a password it did not check. Anything else using the same keychain file, in this process or
    /// another, finds it locked for that moment, and a refused password leaves it locked.
    ///
    /// Apple's implementation adds a newly created keychain to the user's search list (and makes
    /// it the default keychain if the user has none) when its path contains `/login.keychain`.
    /// To make that impossible, and to keep this away from real keychains, `file` refuses with
    /// [`PasskeyError::InvalidInput`], before touching the file system or the Keychain, any path
    /// with a component beginning with `login.keychain` (in any case) and any path inside
    /// `~/Library/Keychains`, `/Library/Keychains` or `/System/Library/Keychains`. Other paths
    /// are created without changing the search list.
    pub fn file(path: &Path, password: &str) -> Result<Self, PasskeyError> {
        check_keychain_path(path)?;
        if path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(PasskeyError::InvalidInput(
                "the keychain path contains a NUL byte",
            ));
        }
        let keychain = if path.exists() {
            let mut keychain = SecKeychain::open(path).map_err(map_error)?;
            lock(&keychain).map_err(map_error)?;
            keychain
                .unlock(Some(password))
                .map_err(|error| match error.code() {
                    ERR_SEC_AUTH_FAILED => PasskeyError::AccessDenied,
                    _ => map_error(error),
                })?;
            keychain
        } else {
            CreateOptions::new()
                .password(password)
                .create(path)
                .map_err(map_error)?
        };
        Ok(Self {
            target: Target::File(keychain),
            access_policy: AccessPolicy::default(),
        })
    }

    /// The same backend with another access policy for the items it adds from now on. Items that
    /// exist keep the access list they were created with.
    pub fn with_access_policy(mut self, access_policy: AccessPolicy) -> Self {
        self.access_policy = access_policy;
        self
    }

    /// The access policy new items are created with.
    pub fn access_policy(&self) -> AccessPolicy {
        self.access_policy
    }

    /// Runs `operation` with Keychain user interaction switched off for the process, so a call
    /// that would show an unlock or access dialog fails with [`PasskeyError::Locked`] instead.
    /// The setting is process-wide: call this only where every Keychain call of the process runs
    /// on the same thread. If interaction is already off (as in tests), it stays off afterwards.
    /// `Err(Locked)` if interaction could not be switched off; `operation` did not run then.
    pub fn without_user_interaction<R>(operation: impl FnOnce() -> R) -> Result<R, PasskeyError> {
        let lock = match SecKeychain::user_interaction_allowed() {
            Ok(false) => None,
            Ok(true) => {
                Some(SecKeychain::disable_user_interaction().map_err(|_| PasskeyError::Locked)?)
            }
            Err(_) => return Err(PasskeyError::Locked),
        };
        let result = operation();
        drop(lock);
        Ok(result)
    }

    fn keychain(&self) -> Result<SecKeychain, PasskeyError> {
        match &self.target {
            Target::Default => SecKeychain::default().map_err(map_error),
            Target::File(keychain) => Ok(keychain.clone()),
        }
    }
}

impl PasskeyBackend for KeychainPasskeyBackend {
    /// `SecItemAdd`; if the item exists, `SecItemUpdate` with the new secret and attributes, which
    /// keeps the creation date. A key whose relying party id or credential id cannot be written
    /// to the account attribute and read back is refused with `InvalidInput`.
    fn put(&self, record: &PasskeyRecord) -> Result<SaveOutcome, PasskeyError> {
        let summary = &record.summary;
        let account = encode_account(&summary.key).ok_or(PasskeyError::InvalidInput(
            "the relying party id or the credential id cannot be stored",
        ))?;
        let comment = encode_metadata(summary).ok_or(PasskeyError::InvalidInput(
            "the passkey's names are too long to store",
        ))?;
        let label = label(summary);
        let secret = SecretData::new(record.private_key.expose()).ok_or(PasskeyError::Internal(
            "a Core Foundation buffer could not be allocated",
        ))?;
        let keychain = self.keychain()?;
        let access = make_access(self.access_policy).map_err(map_error)?;

        match add(&keychain, &access, &account, &label, &comment, &secret) {
            Ok(()) => return Ok(SaveOutcome::Created),
            Err(error) if error.code() == ERR_SEC_DUPLICATE_ITEM => {}
            Err(error) => return Err(map_error(error)),
        }
        let mut update = ItemUpdateOptions::new();
        update
            .set_value(ItemUpdateValue::Data(secret.data.clone()))
            .set_label(&label)
            .set_description(ITEM_DESCRIPTION)
            .set_comment(&comment);
        match update_item(search(&keychain).account(&account), &update) {
            Ok(()) => Ok(SaveOutcome::Replaced),
            // Deleted by someone else between the two calls.
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => {
                add(&keychain, &access, &account, &label, &comment, &secret).map_err(map_error)?;
                Ok(SaveOutcome::Created)
            }
            Err(error) => Err(map_error(error)),
        }
    }

    /// `SecItemAdd` alone: a duplicate is `Exists` and the existing item is not touched.
    fn add(&self, record: &PasskeyRecord) -> Result<AddOutcome, PasskeyError> {
        let summary = &record.summary;
        let account = encode_account(&summary.key).ok_or(PasskeyError::InvalidInput(
            "the relying party id or the credential id cannot be stored",
        ))?;
        let comment = encode_metadata(summary).ok_or(PasskeyError::InvalidInput(
            "the passkey's names are too long to store",
        ))?;
        let label = label(summary);
        let secret = SecretData::new(record.private_key.expose()).ok_or(PasskeyError::Internal(
            "a Core Foundation buffer could not be allocated",
        ))?;
        let keychain = self.keychain()?;
        let access = make_access(self.access_policy).map_err(map_error)?;

        match add(&keychain, &access, &account, &label, &comment, &secret) {
            Ok(()) => Ok(AddOutcome::Added),
            Err(error) if error.code() == ERR_SEC_DUPLICATE_ITEM => Ok(AddOutcome::Exists),
            Err(error) => Err(map_error(error)),
        }
    }

    /// `SecItemCopyMatching` with `kSecReturnData` (the call that can make macOS ask the user),
    /// then one with `kSecReturnAttributes`.
    fn get(&self, key: &PasskeyKey) -> Result<Option<PasskeyRecord>, PasskeyError> {
        let Some(account) = encode_account(key) else {
            return Ok(None);
        };
        let keychain = self.keychain()?;
        let Some(private_key) = read_secret(&keychain, &account)? else {
            return Ok(None);
        };
        let Some(attributes) = read_attributes(&keychain, &account)? else {
            return Ok(None);
        };
        let summary = summarize(&attributes).ok_or(PasskeyError::CorruptItem)?;
        Ok(Some(PasskeyRecord {
            summary,
            private_key,
        }))
    }

    /// `SecItemCopyMatching` for the attributes, then `SecItemUpdate` of the comment only. The
    /// secret is neither read nor written.
    fn set_counter(&self, key: &PasskeyKey, counter: u32) -> Result<bool, PasskeyError> {
        let Some(account) = encode_account(key) else {
            return Ok(false);
        };
        let keychain = self.keychain()?;
        let Some(attributes) = read_attributes(&keychain, &account)? else {
            return Ok(false);
        };
        let mut summary = summarize(&attributes).ok_or(PasskeyError::CorruptItem)?;
        summary.counter = Some(counter);
        let comment = encode_metadata(&summary).ok_or(PasskeyError::CorruptItem)?;
        let mut update = ItemUpdateOptions::new();
        update.set_comment(&comment);
        match update_item(search(&keychain).account(&account), &update) {
            Ok(()) => Ok(true),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(false),
            Err(error) => Err(map_error(error)),
        }
    }

    /// `SecItemDelete`.
    fn delete(&self, key: &PasskeyKey) -> Result<bool, PasskeyError> {
        let Some(account) = encode_account(key) else {
            return Ok(false);
        };
        let keychain = self.keychain()?;
        match search(&keychain).account(&account).delete() {
            Ok(()) => Ok(true),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(false),
            Err(error) => Err(map_error(error)),
        }
    }

    /// `SecItemCopyMatching` with `kSecReturnAttributes` only. Items under the service whose
    /// account or comment does not decode are skipped with a `debug` log line.
    fn list(&self) -> Result<Vec<PasskeySummary>, PasskeyError> {
        let keychain = self.keychain()?;
        let results = match search(&keychain)
            .load_attributes(true)
            .limit(Limit::All)
            .search()
        {
            Ok(results) => results,
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(Vec::new()),
            Err(error) => return Err(map_error(error)),
        };
        let mut summaries = Vec::with_capacity(results.len());
        for result in &results {
            let Some(attributes) = result.simplify_dict() else {
                log::debug!("skipping a Keychain search result that has no attributes");
                continue;
            };
            match summarize(&attributes) {
                Some(summary) => summaries.push(summary),
                None => log::debug!("skipping a Keychain item that is not a version-1 passkey"),
            }
        }
        Ok(summaries)
    }

    /// Like `list`, but a valid version-1 account is enough: an item whose comment is missing or
    /// damaged is still this application's passkey, so its key is returned.
    fn stored_keys(&self) -> Result<Vec<PasskeyKey>, PasskeyError> {
        let keychain = self.keychain()?;
        let results = match search(&keychain)
            .load_attributes(true)
            .limit(Limit::All)
            .search()
        {
            Ok(results) => results,
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(Vec::new()),
            Err(error) => return Err(map_error(error)),
        };
        let mut keys = Vec::with_capacity(results.len());
        for result in &results {
            let key = result.simplify_dict().and_then(|attributes| {
                attributes.get("acct").and_then(|text| decode_account(text))
            });
            match key {
                Some(key) => keys.push(key),
                None => log::debug!("skipping a Keychain item whose account is not a passkey"),
            }
        }
        Ok(keys)
    }
}

/// A `CFData` holding a private key, made here so the bytes can be overwritten once the Security
/// framework call that used it is over (`CFData::from_buffer` makes an immutable copy that can
/// never be wiped).
struct SecretData {
    data: CFData,
    bytes: *mut u8,
    length: usize,
}

impl SecretData {
    fn new(secret: &[u8]) -> Option<Self> {
        let length = isize::try_from(secret.len()).ok()?;
        // SAFETY: `CFDataCreateMutable` returns a new, owned object or null (checked). Its
        // capacity is fixed at `length` (0 means unlimited, and an empty secret appends
        // nothing), so appending exactly `length` bytes from a slice of that length neither
        // reallocates nor reads out of bounds, and the buffer never moves afterwards: this value
        // never resizes the object. `wrap_under_create_rule` takes over the one reference. The
        // mutable and the immutable data types share one layout and one `CFTypeID`.
        unsafe {
            let mutable = CFDataCreateMutable(kCFAllocatorDefault, length);
            if mutable.is_null() {
                return None;
            }
            let data = CFData::wrap_under_create_rule(mutable as CFDataRef);
            if length > 0 {
                CFDataAppendBytes(mutable, secret.as_ptr(), length);
            }
            let bytes = CFDataGetMutableBytePtr(mutable);
            (!bytes.is_null() || length == 0).then_some(Self {
                data,
                bytes,
                length: secret.len(),
            })
        }
    }
}

impl Drop for SecretData {
    fn drop(&mut self) {
        if self.length > 0 && !self.bytes.is_null() {
            // SAFETY: `bytes` points to `length` bytes owned by `self.data`, which is still alive
            // (fields drop after this function) and has not been resized.
            unsafe { std::slice::from_raw_parts_mut(self.bytes, self.length) }.zeroize();
        }
    }
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    /// `OSStatus SecKeychainLock(SecKeychainRef keychain)` from `Security/SecKeychain.h`. Neither
    /// `security-framework` nor `security-framework-sys` declares it. The reference is passed as
    /// the opaque pointer it is.
    fn SecKeychainLock(keychain: CFTypeRef) -> i32;

    /// `kSecAttrAccess` from `Security/SecItem.h` (macOS only): the `SecAccessRef` of a new item.
    /// `security-framework` has no way to set it.
    static kSecAttrAccess: CFStringRef;

    /// `OSStatus SecItemAdd(CFDictionaryRef attributes, CFTypeRef *result)`.
    fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> i32;

    /// `OSStatus SecTrustedApplicationCreateFromPath(const char *path, SecTrustedApplicationRef
    /// *app)` from `Security/SecTrustedApplication.h`. A null path means the calling application.
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, application: *mut CFTypeRef)
    -> i32;

    /// `OSStatus SecAccessCreate(CFStringRef descriptor, CFArrayRef trustedlist, SecAccessRef
    /// *accessRef)` from `Security/SecAccess.h`. The list must not be null (a null list would
    /// trust every application).
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted_list: CFArrayRef,
        access: *mut CFTypeRef,
    ) -> i32;
}

/// Locks `keychain`. Locking a keychain that is already locked succeeds.
fn lock(keychain: &SecKeychain) -> Result<(), SecError> {
    // SAFETY: `keychain` owns a live `SecKeychainRef` for the whole call, and `SecKeychainLock`
    // only reads that reference: it does not keep it, free it or write through it.
    let status = unsafe { SecKeychainLock(keychain.as_CFTypeRef()) };
    if status == 0 {
        Ok(())
    } else {
        Err(SecError::from_code(status))
    }
}

/// The `SecAccess` a new item is created with under `policy`.
fn make_access(policy: AccessPolicy) -> Result<CFType, SecError> {
    match policy {
        AccessPolicy::ThisApplicationOnly => {
            let mut application: CFTypeRef = ptr::null();
            // SAFETY: a null path is allowed and means the calling application; `application` is a
            // valid out pointer. On success the object is owned by the caller (create rule).
            let status =
                unsafe { SecTrustedApplicationCreateFromPath(ptr::null(), &mut application) };
            if status != 0 || application.is_null() {
                return Err(SecError::from_code(if status != 0 { status } else { -50 }));
            }
            // SAFETY: `application` is a non-null, owned reference (checked above).
            let application = unsafe { CFType::wrap_under_create_rule(application) };
            let trusted = CFArray::from_CFTypes(&[application]);
            let descriptor = CFString::new(ITEM_DESCRIPTION);
            let mut access: CFTypeRef = ptr::null();
            // SAFETY: `descriptor` and `trusted` are live CF objects of the types the function
            // takes, and `access` is a valid out pointer; on success the object is owned by the
            // caller (create rule).
            let status = unsafe {
                SecAccessCreate(
                    descriptor.as_concrete_TypeRef(),
                    trusted.as_concrete_TypeRef(),
                    &mut access,
                )
            };
            if status != 0 || access.is_null() {
                return Err(SecError::from_code(if status != 0 { status } else { -50 }));
            }
            // SAFETY: `access` is a non-null, owned reference (checked above).
            Ok(unsafe { CFType::wrap_under_create_rule(access) })
        }
    }
}

/// Refuses paths where creating or opening a keychain could affect the user's real keychains.
///
/// Apple's `SecKeychainCreate` adds a new keychain to the user's search list, and makes it the
/// default keychain if there is none, when its path contains `/login.keychain`. So any path with
/// a component beginning with `login.keychain` (in any case) is refused, as is any path inside
/// `~/Library/Keychains`, `/Library/Keychains` or `/System/Library/Keychains`. Both the path as
/// written (with `.` and `..` resolved) and the path with symbolic links resolved, as far as it
/// exists, are checked; folder names are compared case-insensitively.
fn check_keychain_path(path: &Path) -> Result<(), PasskeyError> {
    let absolute = std::path::absolute(path)
        .map_err(|_| PasskeyError::InvalidInput("the keychain path cannot be resolved"))?;
    let lexical = resolve_dots(&absolute);
    let mut candidates = vec![lexical.clone()];
    if let Some(resolved) = resolve_links(&lexical) {
        candidates.push(resolved);
    }

    let mut roots = vec![
        PathBuf::from("/Library/Keychains"),
        PathBuf::from("/System/Library/Keychains"),
    ];
    if let Some(home) = std::env::home_dir() {
        roots.push(home.join("Library/Keychains"));
    }
    let roots: Vec<PathBuf> = roots
        .iter()
        .flat_map(|root| [Some(root.clone()), root.canonicalize().ok()])
        .flatten()
        .map(|root| lower_case(&root))
        .collect();

    for candidate in &candidates {
        let candidate = lower_case(candidate);
        if candidate.components().any(|component| {
            matches!(component, Component::Normal(name)
                if name.as_encoded_bytes().starts_with(b"login.keychain"))
        }) {
            return Err(PasskeyError::InvalidInput(
                "a keychain file name must not begin with \"login.keychain\"",
            ));
        }
        if roots.iter().any(|root| candidate.starts_with(root)) {
            return Err(PasskeyError::InvalidInput(
                "a keychain file must not be inside a system or user Keychains folder",
            ));
        }
    }
    Ok(())
}

/// `path` without `.` components and with each `..` removing the component before it.
fn resolve_dots(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// `path` with symbolic links resolved in its longest existing ancestor, the rest appended.
fn resolve_links(path: &Path) -> Option<PathBuf> {
    let mut remainder = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            let mut resolved = canonical;
            resolved.extend(remainder.iter().rev());
            return Some(resolved);
        }
        remainder.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

fn lower_case(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

/// A search for this application's generic-password items in `keychain` only.
fn search(keychain: &SecKeychain) -> ItemSearchOptions {
    let mut options = ItemSearchOptions::new();
    options
        .keychains(std::slice::from_ref(keychain))
        .class(ItemClass::generic_password())
        .service(KEYCHAIN_SERVICE);
    options
}

fn add(
    keychain: &SecKeychain,
    access: &CFType,
    account: &str,
    label: &str,
    comment: &str,
    secret: &SecretData,
) -> Result<(), SecError> {
    let mut options = ItemAddOptions::new(ItemAddValue::Data {
        class: ItemClass::generic_password(),
        data: secret.data.clone(),
    });
    options
        .set_location(Location::FileKeychain(keychain.clone()))
        .set_service(KEYCHAIN_SERVICE)
        .set_account_name(account)
        .set_label(label)
        .set_description(ITEM_DESCRIPTION)
        .set_comment(comment);
    // `ItemAddOptions` cannot carry `kSecAttrAccess`, so the dictionary it builds (the method its
    // own `add` calls) gets the access object added, and `SecItemAdd` is called here.
    #[allow(deprecated)]
    let attributes = options.to_dictionary();
    let mut attributes = CFMutableDictionary::from(&attributes);
    // SAFETY: `kSecAttrAccess` is a constant string the framework provides for the life of the
    // process.
    let access_key = unsafe { kSecAttrAccess } as *const c_void;
    attributes.add(&access_key, &access.as_CFTypeRef());
    let attributes = attributes.to_immutable();
    // SAFETY: `attributes` is a live dictionary; a null result pointer is allowed.
    let status = unsafe { SecItemAdd(attributes.as_concrete_TypeRef(), ptr::null_mut()) };
    if status == 0 {
        Ok(())
    } else {
        Err(SecError::from_code(status))
    }
}

/// The private key of the item with this account, or `None`.
fn read_secret(keychain: &SecKeychain, account: &str) -> Result<Option<PrivateKey>, PasskeyError> {
    let results = match search(keychain)
        .account(account)
        .load_data(true)
        .limit(1)
        .search()
    {
        Ok(results) => results,
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(None),
        Err(error) => return Err(map_error(error)),
    };
    match results.into_iter().next() {
        Some(SearchResult::Data(bytes)) => Ok(Some(PrivateKey::new(bytes))),
        Some(_) => Err(PasskeyError::CorruptItem),
        None => Ok(None),
    }
}

/// The attributes of the item with this account, or `None`.
fn read_attributes(
    keychain: &SecKeychain,
    account: &str,
) -> Result<Option<HashMap<String, String>>, PasskeyError> {
    let results = match search(keychain)
        .account(account)
        .load_attributes(true)
        .limit(1)
        .search()
    {
        Ok(results) => results,
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(None),
        Err(error) => return Err(map_error(error)),
    };
    Ok(results
        .into_iter()
        .next()
        .and_then(|result| result.simplify_dict()))
}

/// `v1|<rp id>|<base64url credential id>`; `None` when `key` could not be read back from it (the
/// relying party id is not a lower-case host name, or the credential id is empty or too long).
fn encode_account(key: &PasskeyKey) -> Option<String> {
    if validate_rp_id(&key.rp_id).is_err()
        || key.credential_id.is_empty()
        || key.credential_id.len() > MAX_CREDENTIAL_ID_BYTES
    {
        return None;
    }
    Some(format!(
        "v{FORMAT_VERSION}|{}|{}",
        key.rp_id,
        BASE64URL_NOPAD.encode(&key.credential_id)
    ))
}

/// The inverse of [`encode_account`]; `None` for anything it would not have written.
fn decode_account(account: &str) -> Option<PasskeyKey> {
    let mut parts = account.split('|');
    let (version, rp_id, id) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || version != format!("v{FORMAT_VERSION}") {
        return None;
    }
    let key = PasskeyKey {
        rp_id: rp_id.to_owned(),
        credential_id: BASE64URL_NOPAD.decode(id.as_bytes()).ok()?,
    };
    // Only what `encode_account` writes counts: this refuses non-canonical encodings.
    (encode_account(&key).as_deref() == Some(account)).then_some(key)
}

/// The comment attribute for `summary`; `None` if it would be longer than [`MAX_COMMENT_BYTES`].
fn encode_metadata(summary: &PasskeySummary) -> Option<String> {
    let comment = json!({
        "v": FORMAT_VERSION,
        "uh": summary
            .user_handle
            .as_ref()
            .map(|handle| BASE64URL_NOPAD.encode(handle)),
        "un": summary.username,
        "dn": summary.display_name,
        "c": summary.counter,
        "a": summary.algorithm,
    })
    .to_string();
    (comment.len() <= MAX_COMMENT_BYTES).then_some(comment)
}

/// What the comment attribute holds.
struct Metadata {
    user_handle: Option<Vec<u8>>,
    username: String,
    display_name: String,
    counter: Option<u32>,
    algorithm: i64,
}

/// Reads a comment written by [`encode_metadata`]; unknown extra fields are ignored, a wrong or
/// missing field gives `None`.
fn decode_metadata(text: &str) -> Option<Metadata> {
    if text.len() > MAX_COMMENT_BYTES {
        return None;
    }
    let value: Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    if object.get("v")?.as_u64()? != FORMAT_VERSION {
        return None;
    }
    let user_handle = match object.get("uh")? {
        Value::Null => None,
        Value::String(text) => Some(BASE64URL_NOPAD.decode(text.as_bytes()).ok()?),
        _ => return None,
    };
    let counter = match object.get("c")? {
        Value::Null => None,
        other => Some(u32::try_from(other.as_u64()?).ok()?),
    };
    Some(Metadata {
        user_handle,
        username: object.get("un")?.as_str()?.to_owned(),
        display_name: object.get("dn")?.as_str()?.to_owned(),
        counter,
        algorithm: object.get("a")?.as_i64()?,
    })
}

/// A summary from the attributes of one item (`acct`, `icmt`, `cdat`, `mdat`); `None` if the
/// account or the comment does not decode.
fn summarize(attributes: &HashMap<String, String>) -> Option<PasskeySummary> {
    let key = decode_account(attributes.get("acct")?)?;
    let metadata = decode_metadata(attributes.get("icmt")?)?;
    Some(PasskeySummary {
        key,
        user_handle: metadata.user_handle,
        username: metadata.username,
        display_name: metadata.display_name,
        algorithm: metadata.algorithm,
        counter: metadata.counter,
        created_at_ms: attributes.get("cdat").and_then(|text| parse_date(text)),
        modified_at_ms: attributes.get("mdat").and_then(|text| parse_date(text)),
    })
}

/// `example.com (alice)`, or `example.com` for a passkey without a user name. Control characters
/// become spaces and the name is cut at [`MAX_LABEL_NAME_CHARS`] characters.
fn label(summary: &PasskeySummary) -> String {
    let name: String = summary
        .username
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_LABEL_NAME_CHARS)
        .collect();
    if name.is_empty() {
        summary.key.rp_id.clone()
    } else {
        format!("{} ({name})", summary.key.rp_id)
    }
}

fn map_error(error: SecError) -> PasskeyError {
    match error.code() {
        ERR_SEC_INTERACTION_NOT_ALLOWED => PasskeyError::Locked,
        // A locked keychain, an access list that does not trust this program, and a user who
        // clicked "Deny" all report errSecAuthFailed. With user interaction off the first two
        // are what happened: the system would have had to ask.
        ERR_SEC_AUTH_FAILED => {
            if SecKeychain::user_interaction_allowed().unwrap_or(true) {
                PasskeyError::AccessDenied
            } else {
                PasskeyError::Locked
            }
        }
        ERR_SEC_USER_CANCELED => PasskeyError::AccessDenied,
        ERR_SEC_ITEM_NOT_FOUND => PasskeyError::NotFound,
        code => PasskeyError::Os {
            code,
            message: error
                .message()
                .unwrap_or_else(|| "no description".to_owned()),
        },
    }
}

/// Milliseconds since the Unix epoch of a Keychain date as `SearchResult::simplify_dict` shows
/// it: `2026-11-15 21:16:29 +0000`. Anything else gives `None`.
fn parse_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 25
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b' '
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = text.get(range)?;
        if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let sign = match bytes[20] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let offset_minutes = sign * (number(21..23)? * 60 + number(23..25)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset_minutes * 60;
    seconds.checked_mul(1_000)
}

/// Days from 1970-01-01 to a date in the proleptic Gregorian calendar (the "days from civil"
/// algorithm described by Howard Hinnant, written here from its description).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(rp_id: &str, id: &[u8]) -> PasskeyKey {
        PasskeyKey {
            rp_id: rp_id.to_owned(),
            credential_id: id.to_vec(),
        }
    }

    fn summary(user_handle: Option<Vec<u8>>, counter: Option<u32>) -> PasskeySummary {
        PasskeySummary {
            key: key("example.com", &[7; 32]),
            user_handle,
            username: "Jürgen \"the\" |pipe| 名前".to_owned(),
            display_name: "line\nbreak".to_owned(),
            algorithm: -7,
            counter,
            created_at_ms: None,
            modified_at_ms: None,
        }
    }

    #[test]
    fn keychain_dates_parse_to_unix_milliseconds() {
        assert_eq!(parse_date("1970-01-01 00:00:00 +0000"), Some(0));
        assert_eq!(
            parse_date("2026-11-15 21:16:29 +0000"),
            Some(1_794_777_389_000)
        );
        assert_eq!(
            parse_date("2026-11-15 22:16:29 +0100"),
            Some(1_794_777_389_000)
        );
        assert_eq!(
            parse_date("2024-02-29 12:00:00 +0000"),
            Some(1_709_208_000_000)
        );
    }

    #[test]
    fn other_date_texts_are_unknown() {
        for text in [
            "",
            "2026-11-15",
            "2026-11-15T21:16:29Z",
            "2026-13-15 21:16:29 +0000",
            "2026-11-15 24:16:29 +0000",
            "2026-11-15 21:16:29 0000",
            "2026-1a-15 21:16:29 +0000",
        ] {
            assert_eq!(parse_date(text), None, "{text:?}");
        }
    }

    #[test]
    fn accounts_round_trip_and_are_distinct() {
        let a = key("example.com", &[1; 16]);
        let account = encode_account(&a).expect("encodable");
        assert_eq!(account, "v1|example.com|AQEBAQEBAQEBAQEBAQEBAQ");
        assert_eq!(decode_account(&account), Some(a));
        // The same id on another site, and another id on the same site, are other accounts.
        assert_ne!(Some(account), encode_account(&key("example.org", &[1; 16])));
    }

    #[test]
    fn keys_that_could_not_be_read_back_are_not_encoded() {
        for refused in [
            key("", &[1; 16]),
            key("Example.com", &[1; 16]),
            key("a|b.com", &[1; 16]),
            key("a..b", &[1; 16]),
            key("example.com", &[]),
            key("example.com", &vec![0; MAX_CREDENTIAL_ID_BYTES + 1]),
        ] {
            assert_eq!(encode_account(&refused), None);
        }
        assert!(encode_account(&key("example.com", &vec![0; MAX_CREDENTIAL_ID_BYTES])).is_some());
    }

    #[test]
    fn accounts_this_backend_would_not_write_do_not_decode() {
        for text in [
            "",
            "v1",
            "v1|example.com",
            "v1|example.com|",
            "v2|example.com|AQEBAQEBAQEBAQEBAQEBAQ",
            "v1|Example.com|AQEBAQEBAQEBAQEBAQEBAQ",
            "v1|example.com|AQEBAQEBAQEBAQEBAQEBAQ|extra",
            "v1|example.com|AQEBAQEBAQEBAQEBAQEBAQ==",
            "v1|example.com|AQEBAQEBAQEBAQEBAQEBAR",
            "v1|example.com|not base64!",
        ] {
            assert_eq!(decode_account(text), None, "{text:?}");
        }
    }

    #[test]
    fn metadata_round_trips_hostile_text() {
        for (handle, counter) in [
            (Some(vec![0, 255, 1]), Some(u32::MAX)),
            (None, None),
            (Some(vec![9; 64]), Some(0)),
        ] {
            let original = summary(handle, counter);
            let text = encode_metadata(&original).expect("fits");
            let back = decode_metadata(&text).expect("decodes");
            assert_eq!(back.user_handle, original.user_handle);
            assert_eq!(back.username, original.username);
            assert_eq!(back.display_name, original.display_name);
            assert_eq!(back.counter, original.counter);
            assert_eq!(back.algorithm, -7);
        }
    }

    #[test]
    fn metadata_that_is_not_ours_does_not_decode() {
        for text in [
            "",
            "not json",
            "[]",
            "{}",
            r#"{"v":2,"uh":null,"un":"","dn":"","c":null,"a":-7}"#,
            r#"{"v":1,"uh":5,"un":"","dn":"","c":null,"a":-7}"#,
            r#"{"v":1,"uh":"***","un":"","dn":"","c":null,"a":-7}"#,
            r#"{"v":1,"uh":null,"un":1,"dn":"","c":null,"a":-7}"#,
            r#"{"v":1,"uh":null,"un":"","dn":"","c":-1,"a":-7}"#,
            r#"{"v":1,"uh":null,"un":"","dn":"","c":4294967296,"a":-7}"#,
            r#"{"v":1,"uh":null,"un":"","dn":"","c":null}"#,
        ] {
            assert!(decode_metadata(text).is_none(), "{text:?}");
        }
        assert!(
            decode_metadata(r#"{"v":1,"uh":null,"un":"","dn":"","c":null,"a":-7,"later":[1]}"#)
                .is_some(),
            "fields a later version adds are ignored"
        );
        let long = format!(
            r#"{{"v":1,"uh":null,"un":"{}","dn":"","c":null,"a":-7}}"#,
            "x".repeat(MAX_COMMENT_BYTES)
        );
        assert!(decode_metadata(&long).is_none());
    }

    #[test]
    fn a_comment_that_is_too_long_is_not_written() {
        let mut huge = summary(None, None);
        huge.username = "x".repeat(MAX_COMMENT_BYTES);
        assert_eq!(encode_metadata(&huge), None);
    }

    #[test]
    fn labels_name_the_site_and_user() {
        assert_eq!(
            label(&summary(None, None)),
            "example.com (Jürgen \"the\" |pipe| 名前)"
        );
        let mut plain = summary(None, None);
        plain.username = String::new();
        assert_eq!(label(&plain), "example.com");
        plain.username = "a\u{0}b\nc".to_owned();
        assert_eq!(label(&plain), "example.com (a b c)");
        plain.username = "é".repeat(500);
        assert_eq!(label(&plain), format!("example.com ({})", "é".repeat(100)));
    }

    #[test]
    fn secret_data_holds_the_bytes_and_wipes_them() {
        let secret = SecretData::new(&[0xAB; 48]).expect("allocated");
        assert_eq!(secret.data.bytes(), &[0xAB; 48]);
        let (bytes, length) = (secret.bytes, secret.length);
        let held = secret.data.clone();
        drop(secret);
        // `held` keeps the same CFData alive, so its buffer can be inspected after the wipe.
        assert_eq!(held.bytes(), &[0u8; 48]);
        assert_eq!(length, 48);
        assert!(!bytes.is_null());
        assert!(SecretData::new(&[]).expect("empty is fine").data.is_empty());
    }
}
