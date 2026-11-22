// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Temporary keychains for tests. Every Keychain call in a test goes to a keychain file created
//! here, inside its own temporary directory, and removed when the guard drops — on success and on
//! panic alike. User interaction is switched off for the whole test process before the first
//! Keychain call, so nothing can ever show a dialog: an operation that would have asked the user
//! fails instead.

use std::collections::HashMap;
use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::OnceLock;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::data::{CFData, CFDataRef};
use core_foundation::string::{CFString, CFStringRef};
use mercury_passkeys::keychain::{KEYCHAIN_SERVICE, KeychainPasskeyBackend};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::os::macos::keychain::{
    KeychainSettings, KeychainUserInteractionLock, SecKeychain,
};
use tempfile::TempDir;

/// The invented password of every test keychain.
pub const TEST_KEYCHAIN_PASSWORD: &str = "fake-test-keychain-password";

/// Turns Keychain user interaction off for the rest of this process. The lock lives in a static,
/// so it is never dropped and interaction is never turned back on while tests run.
pub fn no_user_interaction() {
    static LOCK: OnceLock<KeychainUserInteractionLock> = OnceLock::new();
    LOCK.get_or_init(|| {
        SecKeychain::disable_user_interaction().expect("user interaction can be disabled")
    });
    assert!(
        !SecKeychain::user_interaction_allowed().expect("the setting can be read"),
        "Keychain user interaction must be off before any Keychain call"
    );
}

/// A temporary directory holding one test keychain; both are deleted on drop.
pub struct TempKeychain {
    dir: TempDir,
}

impl TempKeychain {
    /// A new, empty directory; the keychain file inside it does not exist yet.
    pub fn new() -> Self {
        no_user_interaction();
        let dir = tempfile::Builder::new()
            .prefix("mercury-passkeys-test-")
            .tempdir()
            .expect("a temporary directory can be created");
        Self { dir }
    }

    /// Where the keychain file lives.
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("test.keychain-db")
    }

    /// The directory the keychain file lives in.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// A backend on this keychain, creating the file on first use.
    pub fn backend(&self) -> KeychainPasskeyBackend {
        KeychainPasskeyBackend::file(&self.path(), TEST_KEYCHAIN_PASSWORD)
            .expect("the test keychain can be created or opened")
    }

    /// A second, independent handle on the same keychain file, for test-only operations.
    pub fn handle(&self) -> SecKeychain {
        no_user_interaction();
        assert!(self.path().exists(), "the keychain must exist already");
        let mut keychain = SecKeychain::open(self.path()).expect("the keychain opens");
        keychain
            .unlock(Some(TEST_KEYCHAIN_PASSWORD))
            .expect("the keychain unlocks");
        keychain
    }
}

/// A guard and a backend on a fresh temporary keychain.
pub fn temp_keychain() -> (TempKeychain, KeychainPasskeyBackend) {
    let guard = TempKeychain::new();
    let backend = guard.backend();
    (guard, backend)
}

/// Attributes of every generic-password item in `keychain`, whatever its service (no secrets are
/// loaded).
pub fn attributes(keychain: &SecKeychain) -> Vec<HashMap<String, String>> {
    match ItemSearchOptions::new()
        .keychains(std::slice::from_ref(keychain))
        .class(ItemClass::generic_password())
        .load_attributes(true)
        .limit(Limit::All)
        .search()
    {
        Ok(items) => items
            .iter()
            .map(|item| item.simplify_dict().expect("attributes are a dictionary"))
            .collect(),
        Err(error) if error.code() == -25300 => Vec::new(),
        Err(error) => panic!("search failed with status {}", error.code()),
    }
}

/// The secret data of the only item in `keychain` whose account is `account`, read straight from
/// the Security framework (not through the backend).
pub fn secret_of(keychain: &SecKeychain, account: &str) -> Vec<u8> {
    let results = ItemSearchOptions::new()
        .keychains(std::slice::from_ref(keychain))
        .class(ItemClass::generic_password())
        .service(KEYCHAIN_SERVICE)
        .account(account)
        .load_data(true)
        .limit(1)
        .search()
        .expect("the item can be read");
    match results.into_iter().next() {
        Some(SearchResult::Data(bytes)) => bytes,
        _ => panic!("the search did not return data"),
    }
}

/// Locks the test keychain and returns a handle that can unlock it again.
///
/// `security-framework` has no call that locks a keychain, so this sets the keychain's idle
/// timer to one second and waits for it, polling instead of sleeping a fixed time: on a loaded
/// machine the timer may fire late. Each probe comes after more than a second of idleness, so even
/// if a probe counts as activity the timer can still fire before the next one. `locked` is a
/// call that fails with the backend's "locked" answer once the keychain is locked (a read of a
/// secret). Gives up after a minute.
pub fn lock_by_idle_timer(guard: &TempKeychain, locked: impl Fn() -> bool) -> SecKeychain {
    let mut handle = guard.handle();
    let mut settings = KeychainSettings::new();
    settings.set_lock_interval(Some(1));
    handle.set_settings(&settings).expect("settings");
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(1_500));
        if locked() {
            return handle;
        }
    }
    panic!("the test keychain did not lock within a minute");
}

/// Unlocks a keychain locked by [`lock_by_idle_timer`] and switches its idle timer off.
pub fn unlock(handle: &mut SecKeychain) {
    handle.unlock(Some(TEST_KEYCHAIN_PASSWORD)).expect("unlock");
    let mut settings = KeychainSettings::new();
    settings.set_lock_interval(None);
    handle.set_settings(&settings).expect("settings");
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainItemCopyAccess(item: CFTypeRef, access: *mut CFTypeRef) -> i32;
    fn SecAccessCopyACLList(access: CFTypeRef, acl_list: *mut CFArrayRef) -> i32;
    fn SecACLCopyAuthorizations(acl: CFTypeRef) -> CFArrayRef;
    fn SecACLCopyContents(
        acl: CFTypeRef,
        application_list: *mut CFArrayRef,
        description: *mut CFStringRef,
        prompt_selector: *mut u32,
    ) -> i32;
    fn SecTrustedApplicationCopyData(application: CFTypeRef, data: *mut CFDataRef) -> i32;
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, application: *mut CFTypeRef)
    -> i32;
}

/// One entry of an item's access control list.
#[derive(Debug)]
pub struct AclEntry {
    /// The operations it governs (`ACLAuthorizationDecrypt`, `ACLAuthorizationAny`, …).
    pub authorizations: Vec<String>,
    /// The prompt description.
    pub description: String,
    /// The applications trusted without a prompt, each as the opaque data the Security framework
    /// keeps for it; `None` means every application is trusted.
    pub applications: Option<Vec<Vec<u8>>>,
}

fn trusted_application_data(application: CFTypeRef) -> Vec<u8> {
    let mut data: CFDataRef = ptr::null();
    // SAFETY: `application` is a live `SecTrustedApplicationRef` and `data` a valid out pointer.
    let status = unsafe { SecTrustedApplicationCopyData(application, &mut data) };
    assert_eq!(status, 0, "SecTrustedApplicationCopyData failed");
    assert!(!data.is_null());
    // SAFETY: the call returned a new, owned CFData.
    let data = unsafe { CFData::wrap_under_create_rule(data) };
    data.bytes().to_vec()
}

/// What the Security framework says about the access control list of the item in `keychain` whose
/// account is `account` (under the passkeys service).
pub fn access_list(keychain: &SecKeychain, account: &str) -> Vec<AclEntry> {
    let item = match ItemSearchOptions::new()
        .keychains(std::slice::from_ref(keychain))
        .class(ItemClass::generic_password())
        .service(KEYCHAIN_SERVICE)
        .account(account)
        .load_refs(true)
        .limit(1)
        .search()
        .expect("the item exists")
        .into_iter()
        .next()
    {
        Some(SearchResult::Ref(Reference::KeychainItem(item))) => item,
        _ => panic!("the search did not return an item reference"),
    };
    let mut access: CFTypeRef = ptr::null();
    // SAFETY: `item` is a live `SecKeychainItemRef`; `access` is a valid out pointer.
    let status = unsafe { SecKeychainItemCopyAccess(item.as_CFTypeRef(), &mut access) };
    assert_eq!(status, 0, "SecKeychainItemCopyAccess failed");
    // SAFETY: the call returned a new, owned `SecAccessRef`.
    let access = unsafe { CFType::wrap_under_create_rule(access) };
    let mut acls: CFArrayRef = ptr::null();
    // SAFETY: `access` is live; `acls` is a valid out pointer.
    let status = unsafe { SecAccessCopyACLList(access.as_CFTypeRef(), &mut acls) };
    assert_eq!(status, 0, "SecAccessCopyACLList failed");
    // SAFETY: the call returned a new, owned array.
    let acls = unsafe { CFArray::<CFType>::wrap_under_create_rule(acls) };

    let mut entries = Vec::new();
    for acl in acls.iter() {
        let acl = acl.as_CFTypeRef();
        // SAFETY: `acl` is a live `SecACLRef`; the call returns a new, owned array or null.
        let authorizations = unsafe { SecACLCopyAuthorizations(acl) };
        let authorizations = if authorizations.is_null() {
            Vec::new()
        } else {
            // SAFETY: owned array of CFStrings.
            let array = unsafe { CFArray::<CFType>::wrap_under_create_rule(authorizations) };
            array
                .iter()
                .filter_map(|tag| tag.downcast::<CFString>())
                .map(|tag| tag.to_string())
                .collect()
        };
        let mut applications: CFArrayRef = ptr::null();
        let mut description: CFStringRef = ptr::null();
        let mut selector = 0u32;
        // SAFETY: `acl` is live; all three out pointers are valid.
        let status =
            unsafe { SecACLCopyContents(acl, &mut applications, &mut description, &mut selector) };
        assert_eq!(status, 0, "SecACLCopyContents failed");
        let applications = if applications.is_null() {
            None
        } else {
            // SAFETY: owned array of `SecTrustedApplicationRef`s.
            let array = unsafe { CFArray::<CFType>::wrap_under_create_rule(applications) };
            Some(
                array
                    .iter()
                    .map(|application| trusted_application_data(application.as_CFTypeRef()))
                    .collect(),
            )
        };
        let description = if description.is_null() {
            String::new()
        } else {
            // SAFETY: owned CFString.
            unsafe { CFString::wrap_under_create_rule(description) }.to_string()
        };
        entries.push(AclEntry {
            authorizations,
            description,
            applications,
        });
    }
    entries
}

/// The data the Security framework keeps for a trusted application made from `path`, or for the
/// running executable when `path` is `None`.
pub fn trusted_application(path: Option<&std::ffi::CStr>) -> Vec<u8> {
    let mut application: CFTypeRef = ptr::null();
    let raw: *const c_char = path.map_or(ptr::null(), std::ffi::CStr::as_ptr);
    // SAFETY: `raw` is null or a NUL-terminated string that outlives the call.
    let status = unsafe { SecTrustedApplicationCreateFromPath(raw, &mut application) };
    assert_eq!(status, 0, "SecTrustedApplicationCreateFromPath failed");
    // SAFETY: the call returned a new, owned reference.
    let application = unsafe { CFType::wrap_under_create_rule(application) };
    trusted_application_data(application.as_CFTypeRef())
}
