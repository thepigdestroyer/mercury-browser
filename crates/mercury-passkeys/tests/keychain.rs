// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The passkey backends: the contract in `common/contract.rs` against `MemoryBackend` everywhere
//! and against the macOS Keychain backend on macOS, then what only the Keychain backend does:
//! item layout, access list, isolation, foreign items, locked keychains and errors. Every Keychain
//! test runs on its own temporary keychain (see `common/keychain.rs`); none touches an existing
//! keychain.

mod common;

use common::contract;
use mercury_passkeys::MemoryBackend;

macro_rules! contract_suite {
    ($suite:ident, $make:expr) => {
        mod $suite {
            use super::*;

            #[test]
            fn empty_backend_lists_nothing() {
                let (_guard, backend) = $make;
                contract::empty_backend_lists_nothing(&backend);
            }

            #[test]
            fn create_read_replace_delete() {
                let (_guard, backend) = $make;
                contract::create_read_replace_delete(&backend);
            }

            #[test]
            fn dates_are_kept_by_the_backend() {
                let (_guard, backend) = $make;
                contract::dates_are_kept_by_the_backend(&backend);
            }

            #[test]
            fn two_passkeys_of_one_site() {
                let (_guard, backend) = $make;
                contract::two_passkeys_of_one_site(&backend);
            }

            #[test]
            fn one_user_on_two_sites() {
                let (_guard, backend) = $make;
                contract::one_user_on_two_sites(&backend);
            }

            #[test]
            fn ids_that_differ_in_the_case_of_their_encoding() {
                let (_guard, backend) = $make;
                contract::ids_that_differ_in_the_case_of_their_encoding(&backend);
            }

            #[test]
            fn set_counter_changes_only_the_counter() {
                let (_guard, backend) = $make;
                contract::set_counter_changes_only_the_counter(&backend);
            }

            #[test]
            fn hostile_values_round_trip() {
                let (_guard, backend) = $make;
                contract::hostile_values_round_trip(&backend);
            }

            #[test]
            fn extreme_sizes_round_trip() {
                let (_guard, backend) = $make;
                contract::extreme_sizes_round_trip(&backend);
            }

            #[test]
            fn add_never_replaces() {
                let (_guard, backend) = $make;
                contract::add_never_replaces(&backend);
            }

            #[test]
            fn bulk_removal_through_the_store() {
                let (_guard, backend) = $make;
                drop(contract::bulk_removal_through_the_store(backend));
            }
        }
    };
}

contract_suite!(memory_backend, ((), MemoryBackend::new()));

#[cfg(target_os = "macos")]
contract_suite!(keychain_backend, common::keychain::temp_keychain());

#[cfg(target_os = "macos")]
mod keychain_only {
    use std::collections::HashMap;

    use super::common::contract::{id, key, passkey, private_key, secret_bytes, summary};
    use super::common::keychain::{
        TEST_KEYCHAIN_PASSWORD, TempKeychain, access_list, attributes, lock_by_idle_timer,
        secret_of, temp_keychain, trusted_application, unlock,
    };
    use core_foundation::data::CFData;
    use data_encoding::{BASE64, BASE64URL, BASE64URL_NOPAD, HEXLOWER, HEXUPPER};
    use mercury_passkeys::keychain::{
        AccessPolicy, ITEM_DESCRIPTION, KEYCHAIN_SERVICE, KeychainPasskeyBackend,
    };
    use mercury_passkeys::{
        PasskeyBackend, PasskeyError, PasskeyKey, PasskeyRecord, PasskeyStore, SaveOutcome,
    };
    use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};
    use security_framework::os::macos::keychain::SecKeychain;

    fn account_of(key: &PasskeyKey) -> String {
        format!(
            "v1|{}|{}",
            key.rp_id,
            BASE64URL_NOPAD.encode(&key.credential_id)
        )
    }

    /// Every spelling of `secret` a careless log line, file or attribute could contain.
    fn forms(secret: &[u8]) -> Vec<Vec<u8>> {
        vec![
            secret.to_vec(),
            HEXLOWER.encode(secret).into_bytes(),
            HEXUPPER.encode(secret).into_bytes(),
            BASE64.encode(secret).into_bytes(),
            BASE64URL.encode(secret).into_bytes(),
            BASE64URL_NOPAD.encode(secret).into_bytes(),
            format!("{secret:?}").into_bytes(),
        ]
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    fn add_foreign_item(
        keychain: &SecKeychain,
        service: &str,
        account: &str,
        comment: Option<&str>,
    ) {
        let mut options = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(b"fake-foreign-secret"),
        });
        options
            .set_location(Location::FileKeychain(keychain.clone()))
            .set_service(service)
            .set_account_name(account)
            .set_label("foreign test item");
        if let Some(comment) = comment {
            options.set_comment(comment);
        }
        options.add().expect("a foreign test item can be added");
    }

    fn item_of(keychain: &SecKeychain, account: &str) -> HashMap<String, String> {
        attributes(keychain)
            .into_iter()
            .find(|item| item.get("acct").map(String::as_str) == Some(account))
            .expect("the item exists")
    }

    #[test]
    fn service_name_is_the_application_bundle_identifier_plus_passkeys() {
        assert_eq!(
            KEYCHAIN_SERVICE,
            "io.github.thepigdestroyer.mercury-browser.passkeys"
        );
    }

    #[test]
    fn one_generic_password_item_per_passkey_with_the_documented_attributes() {
        let (guard, backend) = temp_keychain();
        let alice = passkey("example.com", 1, "alice");
        let mut bob = passkey("login.example.org", 2, "");
        bob.summary.user_handle = None;
        bob.summary.counter = Some(7);
        let carol = passkey("example.com", 3, "carol|%\"x");
        for record in [&alice, &bob, &carol] {
            backend.put(record).expect("save");
        }

        let handle = guard.handle();
        let items = attributes(&handle);
        assert_eq!(items.len(), 3);
        for item in &items {
            assert_eq!(item["class"], "genp");
            assert_eq!(item["svce"], KEYCHAIN_SERVICE);
            assert_eq!(item["desc"], ITEM_DESCRIPTION);
            assert_eq!(item["desc"], "Mercury Browser passkey");
            assert!(item.contains_key("cdat") && item.contains_key("mdat"));
        }

        let item = item_of(&handle, &account_of(&alice.summary.key));
        assert_eq!(item["labl"], "example.com (alice)");
        let comment: serde_json::Value =
            serde_json::from_str(&item["icmt"]).expect("the comment is JSON");
        assert_eq!(
            comment,
            serde_json::json!({
                "v": 1,
                "uh": BASE64URL_NOPAD.encode(b"fake-user-handle"),
                "un": "alice",
                "dn": "Display alice",
                "c": null,
                "a": -7,
            })
        );

        let item = item_of(&handle, &account_of(&bob.summary.key));
        assert_eq!(item["labl"], "login.example.org");
        let comment: serde_json::Value = serde_json::from_str(&item["icmt"]).expect("JSON");
        assert_eq!(comment["uh"], serde_json::Value::Null);
        assert_eq!(comment["c"], 7);

        let item = item_of(&handle, &account_of(&carol.summary.key));
        assert_eq!(item["labl"], "example.com (carol|%\"x)");
    }

    #[test]
    fn the_secret_is_the_item_data_and_appears_nowhere_else() {
        let (guard, backend) = temp_keychain();
        let record = passkey("example.com", 5, "alice");
        backend.put(&record).expect("save");
        let handle = guard.handle();
        let account = account_of(&record.summary.key);

        assert_eq!(secret_of(&handle, &account), secret_bytes(5));

        let secret = secret_bytes(5);
        for item in attributes(&handle) {
            for (name, value) in &item {
                for form in forms(&secret) {
                    assert!(
                        !contains(value.as_bytes(), &form),
                        "attribute {name} holds the secret"
                    );
                }
            }
        }

        // The keychain file holds the attributes in the clear (so this scan can see into it) and
        // the secret only encrypted.
        let mut seen_attribute = false;
        let mut files = 0;
        for entry in std::fs::read_dir(guard.dir()).expect("readable") {
            let path = entry.expect("entry").path();
            if !path.is_file() {
                continue;
            }
            files += 1;
            let bytes = std::fs::read(&path).expect("readable file");
            seen_attribute |= contains(&bytes, b"example.com");
            for form in forms(&secret) {
                assert!(
                    !contains(&bytes, &form),
                    "a file in the keychain's folder holds the secret"
                );
            }
        }
        assert!(files > 0);
        assert!(seen_attribute, "the scan must be able to see attributes");
    }

    #[test]
    fn replacing_rewrites_the_secret_and_attributes_in_one_item() {
        let (guard, backend) = temp_keychain();
        let first = passkey("example.com", 1, "alice");
        backend.put(&first).expect("create");
        let mut second = passkey("example.com", 1, "alice-renamed");
        second.private_key = private_key(8);
        assert_eq!(
            backend.put(&second).expect("replace"),
            SaveOutcome::Replaced
        );

        let handle = guard.handle();
        let items = attributes(&handle);
        assert_eq!(items.len(), 1, "exactly one item");
        assert_eq!(items[0]["labl"], "example.com (alice-renamed)");
        assert!(items[0]["icmt"].contains("alice-renamed"));
        assert_eq!(
            secret_of(&handle, &account_of(&first.summary.key)),
            secret_bytes(8)
        );
    }

    #[test]
    fn keys_that_cannot_be_read_back_are_refused_and_nothing_is_written() {
        let (guard, backend) = temp_keychain();
        for key in [
            key("Example.com", &common_id()),
            key("a|b.example.com", &common_id()),
            key("", &common_id()),
            key("example.com", &[]),
            key("example.com", &vec![1; 1024]),
        ] {
            let record = PasskeyRecord {
                summary: summary(&key.rp_id, &key.credential_id, "alice"),
                private_key: private_key(1),
            };
            assert!(
                matches!(backend.put(&record), Err(PasskeyError::InvalidInput(_))),
                "{key:?}"
            );
        }
        let mut huge = passkey("example.com", 1, "alice");
        huge.summary.username = "x".repeat(20_000);
        assert!(matches!(
            backend.put(&huge),
            Err(PasskeyError::InvalidInput(_))
        ));
        assert!(attributes(&guard.handle()).is_empty());
    }

    fn common_id() -> Vec<u8> {
        id(1)
    }

    /// Paths `KeychainPasskeyBackend::file` must refuse before touching the file system or the
    /// Keychain. Nothing is created: each path is refused, and the test checks it still does not
    /// exist.
    #[test]
    fn login_keychain_names_and_real_keychain_folders_are_refused() {
        let guard = TempKeychain::new();
        let home = std::env::home_dir().expect("the test user has a home directory");
        let refused = [
            guard.dir().join("login.keychain-db"),
            guard.dir().join("login.keychain"),
            guard.dir().join("LOGIN.KEYCHAIN-DB"),
            guard.dir().join("Login.Keychain.copy"),
            guard
                .dir()
                .join("login.keychain-folder")
                .join("other.keychain-db"),
            home.join("Library/Keychains/mercury-test-never-created.keychain-db"),
            home.join("Library/Keychains/sub/mercury-test-never-created.keychain-db"),
            home.join("library/KEYCHAINS/mercury-test-never-created.keychain-db"),
            home.join("Library/Other/../Keychains/mercury-test-never-created.keychain-db"),
            "/Library/Keychains/mercury-test-never-created.keychain-db".into(),
            "/Library/Keychains/./mercury-test-never-created.keychain-db".into(),
            "/System/Library/Keychains/mercury-test-never-created.keychain-db".into(),
        ];
        for path in &refused {
            assert!(
                !path.exists(),
                "{} must not exist before the test",
                path.display()
            );
            match KeychainPasskeyBackend::file(path, TEST_KEYCHAIN_PASSWORD) {
                Err(PasskeyError::InvalidInput(reason)) => {
                    assert!(reason.contains("keychain"), "{reason}")
                }
                Err(other) => panic!("{} gave {other:?}", path.display()),
                Ok(_) => panic!("{} was accepted", path.display()),
            }
            assert!(
                !path.exists(),
                "{} must not have been created",
                path.display()
            );
        }
        assert!(
            std::fs::read_dir(guard.dir())
                .expect("the temporary directory is readable")
                .next()
                .is_none(),
            "nothing was created in the temporary directory"
        );
    }

    #[test]
    fn ordinary_names_in_a_temporary_folder_are_accepted() {
        let guard = TempKeychain::new();
        for name in [
            "my-login.keychain-db",
            "logins.keychain-db",
            "test.keychain-db",
        ] {
            let path = guard.dir().join(name);
            drop(KeychainPasskeyBackend::file(&path, TEST_KEYCHAIN_PASSWORD).expect("accepted"));
            assert!(path.exists());
        }
    }

    #[test]
    fn a_keychain_file_is_created_once_and_reopened_with_its_password() {
        let guard = TempKeychain::new();
        assert!(!guard.path().exists());
        let first = guard.backend();
        assert!(guard.path().exists());
        let record = passkey("example.com", 1, "alice");
        first.put(&record).expect("save");
        drop(first);

        let reopened =
            KeychainPasskeyBackend::file(&guard.path(), TEST_KEYCHAIN_PASSWORD).expect("reopen");
        let back = reopened
            .get(&record.summary.key)
            .expect("read")
            .expect("present");
        assert!(back.private_key.same_as(&record.private_key));
    }

    #[test]
    fn a_wrong_keychain_password_is_refused_without_naming_it() {
        let guard = TempKeychain::new();
        drop(guard.backend());
        let error = match KeychainPasskeyBackend::file(&guard.path(), "fake-wrong-password") {
            Ok(_) => panic!("a wrong password must not open the keychain"),
            Err(error) => error,
        };
        assert_eq!(error, PasskeyError::AccessDenied);
        assert!(!format!("{error} {error:?}").contains("fake-wrong-password"));
    }

    /// macOS checks an unlock password only when the keychain is locked, so refusing a wrong one
    /// needs `file` to lock the keychain first. This holds the keychain unlocked through a second
    /// handle and tries a wrong password twice (the first refusal leaves the keychain locked, the
    /// second meets it locked); the right password must still open it, with its items intact.
    #[test]
    fn a_wrong_password_is_refused_every_time_and_the_right_one_still_opens_the_keychain() {
        let (guard, backend) = temp_keychain();
        let record = passkey("example.com", 1, "alice");
        backend.put(&record).expect("save");
        drop(backend);
        let _unlocked_elsewhere = guard.handle();

        for _ in 0..2 {
            match KeychainPasskeyBackend::file(&guard.path(), "fake-wrong-password") {
                Err(error) => assert_eq!(error, PasskeyError::AccessDenied),
                Ok(_) => panic!("a wrong password must not open the keychain"),
            }
        }

        let reopened =
            KeychainPasskeyBackend::file(&guard.path(), TEST_KEYCHAIN_PASSWORD).expect("reopen");
        assert!(reopened.get(&record.summary.key).expect("read").is_some());
    }

    #[test]
    fn operations_touch_only_the_targeted_keychain() {
        let (first_guard, first) = temp_keychain();
        let (second_guard, second) = temp_keychain();
        let mut in_first = passkey("example.com", 1, "alice");
        in_first.private_key = private_key(31);
        let mut in_second = passkey("example.com", 1, "alice");
        in_second.private_key = private_key(32);
        first.put(&in_first).expect("save first");
        second.put(&in_second).expect("save second");
        let key = in_first.summary.key.clone();

        let mut changed = passkey("example.com", 1, "alice");
        changed.private_key = private_key(33);
        assert_eq!(first.put(&changed).expect("update"), SaveOutcome::Replaced);
        assert!(first.set_counter(&key, 4).expect("counter"));
        assert!(
            second
                .get(&key)
                .expect("read")
                .expect("present")
                .private_key
                .same_as(&private_key(32))
        );
        assert_eq!(second.list().expect("list")[0].counter, None);
        assert!(first.delete(&key).expect("delete"));
        assert!(second.get(&key).expect("read").is_some());
        assert_eq!(second.list().expect("list").len(), 1);
        assert!(first.list().expect("list").is_empty());
        assert!(attributes(&first_guard.handle()).is_empty());
        assert_eq!(attributes(&second_guard.handle()).len(), 1);
    }

    #[test]
    fn other_items_are_skipped_never_listed_or_removed() {
        let (guard, backend) = temp_keychain();
        let handle = guard.handle();
        let good_comment = r#"{"v":1,"uh":null,"un":"","dn":"","c":null,"a":-7}"#;
        let id_text = BASE64URL_NOPAD.encode(&id(1));
        add_foreign_item(
            &handle,
            "com.example.other-app",
            &format!("v1|example.com|{id_text}"),
            Some(good_comment),
        );
        for account in [
            "not a passkey".to_owned(),
            format!("v2|example.com|{id_text}"),
            format!("v1|EXAMPLE.com|{id_text}"),
            format!("v1|example.com|{id_text}|extra"),
            format!("v1|example.com|{id_text}=="),
            "v1|example.com|".to_owned(),
            format!("v1||{id_text}"),
            format!("v1|a b.com|{id_text}"),
        ] {
            add_foreign_item(&handle, KEYCHAIN_SERVICE, &account, Some(good_comment));
        }
        // A readable account with no comment, and with a comment of another shape.
        add_foreign_item(
            &handle,
            KEYCHAIN_SERVICE,
            &format!("v1|no-comment.example|{id_text}"),
            None,
        );
        add_foreign_item(
            &handle,
            KEYCHAIN_SERVICE,
            &format!("v1|bad-comment.example|{id_text}"),
            Some("not json"),
        );
        let foreign = attributes(&handle).len();
        // One other application's item, eight undecodable accounts and two undecodable comments.
        assert_eq!(foreign, 11);

        let store = PasskeyStore::new(backend);
        store
            .save(&passkey("example.com", 1, "alice"))
            .expect("save");
        store.save(&passkey("example.org", 2, "bob")).expect("save");
        assert_eq!(store.all().expect("all").len(), 2);

        assert_eq!(store.remove_all().expect("remove all"), 2);
        assert!(store.all().expect("all").is_empty());
        let remaining = attributes(&handle);
        assert_eq!(
            remaining.len(),
            foreign,
            "every foreign item is still there"
        );
        assert!(
            remaining
                .iter()
                .all(|item| item["labl"] == "foreign test item")
        );
    }

    #[test]
    fn an_item_with_a_corrupt_comment_is_skipped_by_list_and_reported_by_get_and_set_counter() {
        let (guard, backend) = temp_keychain();
        let broken = key("broken.example", &id(1));
        let handle = guard.handle();
        add_foreign_item(
            &handle,
            KEYCHAIN_SERVICE,
            &account_of(&broken),
            Some("{\"v\":1}"),
        );
        let healthy = passkey("example.com", 2, "alice");
        backend.put(&healthy).expect("save");

        let listed = backend.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, healthy.summary.key);
        assert!(matches!(
            backend.get(&broken),
            Err(PasskeyError::CorruptItem)
        ));
        assert!(matches!(
            backend.set_counter(&broken, 1),
            Err(PasskeyError::CorruptItem)
        ));
        assert_eq!(attributes(&handle).len(), 2, "never removed by listing");

        // Saving the passkey again repairs the item.
        let mut repaired = passkey("broken.example", 1, "alice");
        repaired.summary.key = broken.clone();
        assert_eq!(backend.put(&repaired).expect("put"), SaveOutcome::Replaced);
        assert_eq!(backend.list().expect("list").len(), 2);
        assert!(backend.get(&broken).expect("get").is_some());
    }

    /// With the keychain locked and user interaction off, `list` still answers (it reads
    /// attributes only, which a locked file keychain serves), while reading a secret, adding an
    /// item and replacing one fail with `Locked`.
    #[test]
    fn a_locked_keychain_lists_but_cannot_read_or_write_secrets() {
        let (guard, backend) = temp_keychain();
        let record = passkey("example.com", 1, "locked-user");
        backend.put(&record).expect("save");
        let key = record.summary.key.clone();
        let mut handle = lock_by_idle_timer(&guard, || match backend.get(&key) {
            Err(PasskeyError::Locked) => true,
            Ok(_) => false,
            Err(other) => panic!("unexpected error while waiting for the lock: {other:?}"),
        });

        let error = backend
            .get(&key)
            .expect_err("a locked keychain cannot be read");
        assert_eq!(error, PasskeyError::Locked);
        assert!(!format!("{error} {error:?}").contains("locked-user"));
        assert_eq!(
            backend
                .put(&passkey("example.org", 2, "new"))
                .expect_err("add"),
            PasskeyError::Locked
        );
        let mut replacement = passkey("example.com", 1, "locked-user");
        replacement.private_key = private_key(77);
        assert_eq!(
            backend.put(&replacement).expect_err("replace"),
            PasskeyError::Locked
        );
        let listed = backend.list().expect("listing reads attributes only");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, key);
        assert_eq!(listed[0].username, "locked-user");
        assert_eq!(attributes(&handle).len(), 1, "exactly one item");

        unlock(&mut handle);
        assert!(
            backend
                .get(&key)
                .expect("read")
                .expect("present")
                .private_key
                .same_as(&record.private_key),
            "the failed replacement changed nothing"
        );
    }

    #[test]
    fn the_item_is_created_with_an_explicit_access_list_naming_only_this_application() {
        let (guard, backend) = temp_keychain();
        assert_eq!(backend.access_policy(), AccessPolicy::ThisApplicationOnly);
        let record = passkey("example.com", 1, "alice");
        backend.put(&record).expect("save");
        let entries = access_list(&guard.handle(), &account_of(&record.summary.key));
        assert!(!entries.is_empty());

        let this_application = trusted_application(None);
        let other_application = trusted_application(Some(c"/bin/ls"));
        assert_ne!(this_application, other_application);

        let decrypting: Vec<_> = entries
            .iter()
            .filter(|entry| {
                entry
                    .authorizations
                    .iter()
                    .any(|tag| tag == "ACLAuthorizationDecrypt" || tag == "ACLAuthorizationAny")
            })
            .collect();
        assert!(!decrypting.is_empty(), "{entries:?}");
        for entry in decrypting {
            let applications = entry
                .applications
                .as_ref()
                .unwrap_or_else(|| panic!("an entry trusts every application: {entry:?}"));
            assert_eq!(applications, &vec![this_application.clone()], "{entry:?}");
            assert!(!applications.contains(&other_application));
        }
    }

    #[test]
    fn debug_output_and_errors_hold_no_secret() {
        let (guard, backend) = temp_keychain();
        let record = passkey("example.com", 6, "debug-user-name");
        backend.put(&record).expect("save");
        let secret = secret_bytes(6);

        let mut texts = vec![
            format!("{backend:?}"),
            format!("{:?}", KeychainPasskeyBackend::login()),
            format!("{record:?}"),
            format!("{:?}", record.private_key),
            format!("{:?}", backend.get(&record.summary.key)),
            format!("{:?}", backend.list()),
        ];
        let mut huge = passkey("example.com", 6, "debug-user-name");
        huge.summary.username = "x".repeat(20_000);
        let errors = [
            backend.put(&huge).expect_err("too long"),
            backend
                .put(&passkey("Bad|Site", 6, "debug-user-name"))
                .expect_err("bad site"),
            KeychainPasskeyBackend::file(&guard.path(), "fake-wrong-password")
                .expect_err("wrong password"),
        ];
        for error in &errors {
            texts.push(format!("{error} | {error:?}"));
        }
        texts.push(format!("{:?}", backend.access_policy()));
        for text in &texts {
            for form in forms(&secret) {
                assert!(
                    !contains(text.as_bytes(), &form),
                    "a Debug text holds the secret"
                );
            }
            assert!(!text.contains(TEST_KEYCHAIN_PASSWORD));
            assert!(!text.contains("fake-wrong-password"));
        }
        // The backend's own Debug names neither the file nor any passkey.
        let shown = format!("{backend:?}");
        assert!(!shown.contains("test.keychain"));
        assert!(!shown.contains("example.com"));
        assert!(!shown.contains("debug-user-name"));
        assert!(shown.contains("KeychainPasskeyBackend"));
        assert!(shown.contains("ThisApplicationOnly"));
        // The credential id never appears in a passkey summary's Debug either.
        assert!(!format!("{record:?}").contains(&BASE64URL_NOPAD.encode(&id(6))));
    }

    #[test]
    fn user_interaction_stays_off_around_without_user_interaction() {
        let _guard = TempKeychain::new();
        let value = KeychainPasskeyBackend::without_user_interaction(|| 41 + 1)
            .expect("interaction is already off, so this cannot fail");
        assert_eq!(value, 42);
        assert!(!SecKeychain::user_interaction_allowed().expect("readable"));
    }
}
