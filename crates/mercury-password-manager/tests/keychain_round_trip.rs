// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Import, edit and export against a real Keychain backend, on a temporary keychain file only.
//! User interaction is switched off for the whole process before the first Keychain call, so no
//! dialog can appear; the keychain's directory is deleted when the test ends, on panic too.

#![cfg(target_os = "macos")]

use std::sync::OnceLock;

use mercury_password_manager::{AnyBackend, export_csv, import_csv, login_rows, update_login};
use mercury_passwords::{KeychainBackend, PasswordStore};
use security_framework::os::macos::keychain::{KeychainUserInteractionLock, SecKeychain};

const TEST_KEYCHAIN_PASSWORD: &str = "fake-test-keychain-password";

fn no_user_interaction() {
    static LOCK: OnceLock<KeychainUserInteractionLock> = OnceLock::new();
    LOCK.get_or_init(|| {
        SecKeychain::disable_user_interaction().expect("user interaction can be disabled")
    });
    assert!(!SecKeychain::user_interaction_allowed().expect("the setting can be read"));
}

#[test]
fn logins_survive_import_edit_and_export_through_a_temporary_keychain() {
    no_user_interaction();
    let directory = tempfile::Builder::new()
        .prefix("mercury-password-manager-test-")
        .tempdir()
        .expect("a temporary directory");
    let backend = KeychainBackend::file(
        &directory.path().join("test.keychain-db"),
        TEST_KEYCHAIN_PASSWORD,
    )
    .expect("the test keychain");
    let store = PasswordStore::new(AnyBackend::keychain(backend));
    assert!(store.backend().is_persistent());

    let report = import_csv(
        &store,
        "name,url,username,password\n\
         a.test,https://a.test/,alice,fake-a\n\
         b.test,https://b.test/,bob,fake-b\n",
    )
    .expect("imported");
    assert_eq!(report.added, 2);

    let rows = login_rows(&store.all().expect("listed"));
    assert_eq!(rows.len(), 2);
    update_login(&store, &rows[1].key, "bobby", None).expect("renamed");

    // Quiet mode (no dialog may appear) still reads what this program saved.
    store.backend().set_quiet(true);
    assert_eq!(store.all().expect("listed quietly").len(), 2);
    store.backend().set_quiet(false);

    let exported = export_csv(&store).expect("exported");
    assert_eq!(
        exported.text.as_str(),
        "name,url,username,password,note\n\
         a.test,https://a.test,alice,fake-a,\n\
         b.test,https://b.test,bobby,fake-b,\n"
    );
}
