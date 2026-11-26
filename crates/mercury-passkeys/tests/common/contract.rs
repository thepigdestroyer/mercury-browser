// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The contract every `PasskeyBackend` must meet. `tests/keychain.rs` runs each function against
//! every backend. All keys here are obviously fake byte patterns, not real private keys.

use std::thread::sleep;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use mercury_passkeys::testing::FakeVerifier;
use mercury_passkeys::{
    AddOutcome, MAX_CREDENTIAL_ID_BYTES, MAX_NAME_BYTES, MAX_PRIVATE_KEY_BYTES, PasskeyBackend,
    PasskeyKey, PasskeyRecord, PasskeyStore, PasskeySummary, PrivateKey, SaveOutcome,
};

pub fn key(rp_id: &str, credential_id: &[u8]) -> PasskeyKey {
    PasskeyKey {
        rp_id: rp_id.to_owned(),
        credential_id: credential_id.to_vec(),
    }
}

/// A 32-byte credential id that differs for every `seed`.
pub fn id(seed: u8) -> Vec<u8> {
    (0..32u8)
        .map(|index| seed.wrapping_mul(37).wrapping_add(index))
        .collect()
}

/// 48 bytes standing in for a private COSE_Key, different for every `seed` and recognisable in a
/// dump.
pub fn secret_bytes(seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xD7, 0x5E, 0xC4];
    bytes.extend((0..45u8).map(|index| seed.wrapping_mul(91).wrapping_add(index.wrapping_mul(7))));
    bytes
}

pub fn private_key(seed: u8) -> PrivateKey {
    PrivateKey::new(secret_bytes(seed))
}

pub fn summary(rp_id: &str, credential_id: &[u8], username: &str) -> PasskeySummary {
    PasskeySummary {
        key: key(rp_id, credential_id),
        user_handle: Some(b"fake-user-handle".to_vec()),
        username: username.to_owned(),
        display_name: format!("Display {username}"),
        algorithm: -7,
        counter: None,
        created_at_ms: None,
        modified_at_ms: None,
    }
}

/// A passkey for `rp_id` with credential id `id(id_seed)` and the secret `secret_bytes(id_seed)`.
pub fn passkey(rp_id: &str, id_seed: u8, username: &str) -> PasskeyRecord {
    PasskeyRecord {
        summary: summary(rp_id, &id(id_seed), username),
        private_key: private_key(id_seed),
    }
}

/// Whether two summaries agree on everything a backend stores except its dates.
pub fn same_fields(a: &PasskeySummary, b: &PasskeySummary) -> bool {
    a.key == b.key
        && a.user_handle == b.user_handle
        && a.username == b.username
        && a.display_name == b.display_name
        && a.algorithm == b.algorithm
        && a.counter == b.counter
}

fn read(backend: &impl PasskeyBackend, key: &PasskeyKey) -> Option<PasskeyRecord> {
    backend.get(key).expect("reading a passkey should work")
}

fn sorted_keys(backend: &impl PasskeyBackend) -> Vec<PasskeyKey> {
    let mut keys: Vec<PasskeyKey> = backend
        .list()
        .expect("listing should work")
        .into_iter()
        .map(|summary| summary.key)
        .collect();
    keys.sort_by(|a, b| (&a.rp_id, &a.credential_id).cmp(&(&b.rp_id, &b.credential_id)));
    keys
}

fn listed(backend: &impl PasskeyBackend, key: &PasskeyKey) -> PasskeySummary {
    backend
        .list()
        .expect("listing should work")
        .into_iter()
        .find(|summary| &summary.key == key)
        .expect("the passkey should be listed")
}

pub fn empty_backend_lists_nothing(backend: &impl PasskeyBackend) {
    assert!(backend.list().expect("list").is_empty());
    let never_stored = [
        key("example.com", &id(1)),
        key("evil|x.com", &id(1)),
        key("UPPER.com", &id(1)),
        key("", &id(1)),
        key("example.com", &[]),
        key("example.com", &vec![1; MAX_CREDENTIAL_ID_BYTES + 1]),
    ];
    for key in &never_stored {
        assert!(read(backend, key).is_none());
        assert!(!backend.delete(key).expect("delete works"));
        assert!(!backend.set_counter(key, 3).expect("set_counter works"));
    }
}

pub fn create_read_replace_delete(backend: &impl PasskeyBackend) {
    let first = passkey("example.com", 1, "alice");
    let key = first.summary.key.clone();

    assert_eq!(backend.put(&first).expect("create"), SaveOutcome::Created);
    let back = read(backend, &key).expect("the passkey is stored");
    assert!(same_fields(&back.summary, &first.summary));
    assert!(back.private_key.same_as(&first.private_key));
    let shown = listed(backend, &key);
    assert!(same_fields(&shown, &first.summary));
    assert_eq!(sorted_keys(backend), vec![key.clone()]);

    let mut second = passkey("example.com", 1, "alice-renamed");
    second.private_key = private_key(2);
    second.summary.user_handle = None;
    second.summary.display_name = String::new();
    assert_eq!(
        backend.put(&second).expect("replace"),
        SaveOutcome::Replaced
    );
    let back = read(backend, &key).expect("the passkey is stored");
    assert!(same_fields(&back.summary, &second.summary));
    assert!(back.private_key.same_as(&private_key(2)));
    assert!(!back.private_key.same_as(&first.private_key));
    assert_eq!(sorted_keys(backend), vec![key.clone()]);

    assert!(backend.delete(&key).expect("delete"));
    assert!(read(backend, &key).is_none());
    assert!(backend.list().expect("list").is_empty());
    assert!(!backend.delete(&key).expect("second delete"));
}

pub fn dates_are_kept_by_the_backend(backend: &impl PasskeyBackend) {
    let record = passkey("example.com", 1, "alice");
    let key = record.summary.key.clone();
    backend.put(&record).expect("create");
    let created = listed(backend, &key);
    let created_at = created.created_at_ms.expect("creation time is known");
    let modified_at = created.modified_at_ms.expect("modification time is known");
    assert!(
        created_at > 1_700_000_000_000,
        "a plausible Unix time in milliseconds"
    );
    assert!(modified_at >= created_at);

    // The Keychain stores times to the second; wait long enough for a rewrite to show.
    sleep(Duration::from_millis(1_100));
    let mut changed = passkey("example.com", 1, "alice-renamed");
    changed.private_key = private_key(9);
    backend.put(&changed).expect("replace");
    let replaced = listed(backend, &key);
    assert_eq!(
        replaced.created_at_ms,
        Some(created_at),
        "replacing keeps the creation date"
    );
    assert!(replaced.modified_at_ms.expect("known") > modified_at);
    let record = read(backend, &key).expect("present");
    assert_eq!(record.summary.created_at_ms, Some(created_at));
}

pub fn two_passkeys_of_one_site(backend: &impl PasskeyBackend) {
    let alice = passkey("example.com", 1, "alice");
    let bob = passkey("example.com", 2, "bob");
    assert_eq!(backend.put(&alice).expect("alice"), SaveOutcome::Created);
    assert_eq!(backend.put(&bob).expect("bob"), SaveOutcome::Created);
    assert_eq!(sorted_keys(backend).len(), 2);

    let back = read(backend, &alice.summary.key).expect("alice");
    assert_eq!(back.summary.username, "alice");
    assert!(back.private_key.same_as(&alice.private_key));
    let back = read(backend, &bob.summary.key).expect("bob");
    assert_eq!(back.summary.username, "bob");
    assert!(back.private_key.same_as(&bob.private_key));

    assert!(backend.delete(&alice.summary.key).expect("delete alice"));
    assert!(read(backend, &alice.summary.key).is_none());
    let back = read(backend, &bob.summary.key).expect("bob is untouched");
    assert!(back.private_key.same_as(&bob.private_key));
    assert_eq!(sorted_keys(backend), vec![bob.summary.key.clone()]);
}

/// The same credential id and user on two sites are two passkeys.
pub fn one_user_on_two_sites(backend: &impl PasskeyBackend) {
    let mut on_example = passkey("example.com", 1, "alice");
    let mut on_other = passkey("login.example.org", 1, "alice");
    on_example.private_key = private_key(11);
    on_other.private_key = private_key(12);
    backend.put(&on_example).expect("first site");
    backend.put(&on_other).expect("second site");
    assert_eq!(sorted_keys(backend).len(), 2);
    assert!(
        read(backend, &on_example.summary.key)
            .expect("first")
            .private_key
            .same_as(&private_key(11))
    );
    assert!(
        read(backend, &on_other.summary.key)
            .expect("second")
            .private_key
            .same_as(&private_key(12))
    );
    assert!(backend.delete(&on_example.summary.key).expect("delete"));
    assert!(read(backend, &on_other.summary.key).is_some());
    // A relying party id is matched exactly, never as a parent domain.
    assert!(read(backend, &key("example.org", &id(1))).is_none());
    assert!(read(backend, &key("example.com", &id(1))).is_none());
}

/// Credential ids whose base64url texts differ only in letter case are different passkeys (a
/// case-insensitive match on the account would merge them).
pub fn ids_that_differ_in_the_case_of_their_encoding(backend: &impl PasskeyBackend) {
    let upper = BASE64URL_NOPAD
        .decode("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".as_bytes())
        .expect("valid base64url");
    let lower = BASE64URL_NOPAD
        .decode("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".as_bytes())
        .expect("valid base64url");
    assert_ne!(upper, lower);
    let mut first = passkey("example.com", 1, "upper");
    first.summary.key.credential_id = upper;
    first.private_key = private_key(21);
    let mut second = passkey("example.com", 1, "lower");
    second.summary.key.credential_id = lower;
    second.private_key = private_key(22);

    assert_eq!(backend.put(&first).expect("first"), SaveOutcome::Created);
    assert_eq!(backend.put(&second).expect("second"), SaveOutcome::Created);
    assert_eq!(sorted_keys(backend).len(), 2);
    assert_eq!(
        read(backend, &first.summary.key)
            .expect("first")
            .summary
            .username,
        "upper"
    );
    assert_eq!(
        read(backend, &second.summary.key)
            .expect("second")
            .summary
            .username,
        "lower"
    );
    assert!(backend.delete(&first.summary.key).expect("delete"));
    assert!(read(backend, &second.summary.key).is_some());
}

pub fn set_counter_changes_only_the_counter(backend: &impl PasskeyBackend) {
    let record = passkey("example.com", 1, "alice");
    let key = record.summary.key.clone();
    backend.put(&record).expect("create");
    let before = listed(backend, &key);
    assert_eq!(before.counter, None);

    assert!(backend.set_counter(&key, 5).expect("set"));
    let after = read(backend, &key).expect("present");
    assert_eq!(after.summary.counter, Some(5));
    assert!(after.private_key.same_as(&record.private_key));
    assert_eq!(after.summary.username, "alice");
    assert_eq!(after.summary.user_handle, record.summary.user_handle);
    assert_eq!(after.summary.created_at_ms, before.created_at_ms);
    assert_eq!(listed(backend, &key).counter, Some(5));

    assert!(backend.set_counter(&key, u32::MAX).expect("set again"));
    assert_eq!(listed(backend, &key).counter, Some(u32::MAX));
    assert!(backend.set_counter(&key, 0).expect("set zero"));
    assert_eq!(listed(backend, &key).counter, Some(0));

    let other = PasskeyKey {
        rp_id: "example.com".to_owned(),
        credential_id: id(99),
    };
    assert!(!backend.set_counter(&other, 1).expect("missing"));
    assert!(read(backend, &other).is_none());
}

pub fn hostile_values_round_trip(backend: &impl PasskeyBackend) {
    let long_name = "é".repeat(MAX_NAME_BYTES / 2);
    assert_eq!(long_name.len(), MAX_NAME_BYTES);
    let names = [
        "",
        "  spaced  name  ",
        "tab\there",
        "quote\"s and 'apostrophes' and \\backslashes\\",
        "pipe|percent%25|%7C and v1|example.com|x",
        "{\"v\":1,\"uh\":null}",
        "Jürgen 名前 🔑",
        "line\nbreak",
        long_name.as_str(),
    ];
    let mut expected = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let seed = u8::try_from(index).expect("few names") + 40;
        let mut record = passkey("example.com", seed, name);
        record.summary.display_name = format!("{name}{name}");
        record.summary.user_handle = if index % 2 == 0 {
            Some(vec![0u8; 64])
        } else {
            None
        };
        record.summary.counter = [None, Some(0), Some(u32::MAX)][index % 3];
        record.summary.algorithm = [-7, -8, -257][index % 3];
        assert_eq!(
            backend.put(&record).expect("hostile values are stored"),
            SaveOutcome::Created,
            "name {index} must be a new passkey"
        );
        expected.push(record);
    }
    assert_eq!(sorted_keys(backend).len(), names.len());
    for record in &expected {
        let back = read(backend, &record.summary.key).expect("stored");
        assert!(same_fields(&back.summary, &record.summary));
        assert!(back.private_key.same_as(&record.private_key));
        assert!(same_fields(
            &listed(backend, &record.summary.key),
            &record.summary
        ));
    }
}

pub fn extreme_sizes_round_trip(backend: &impl PasskeyBackend) {
    let mut longest_id = passkey("a.b.c.example.com", 1, "alice");
    longest_id.summary.key.credential_id = (0..MAX_CREDENTIAL_ID_BYTES)
        .map(|index| u8::try_from(index % 251).expect("small"))
        .collect();
    let mut shortest_id = passkey("example.com", 2, "bob");
    shortest_id.summary.key.credential_id = vec![0xFF];
    let mut biggest_key = passkey("example.com", 3, "carol");
    biggest_key.private_key = PrivateKey::new(
        (0..MAX_PRIVATE_KEY_BYTES)
            .map(|index| u8::try_from(index % 253).expect("small"))
            .collect(),
    );
    let mut smallest_key = passkey("example.com", 4, "dave");
    smallest_key.private_key = PrivateKey::new(vec![0x80]);

    for record in [&longest_id, &shortest_id, &biggest_key, &smallest_key] {
        backend.put(record).expect("stored");
    }
    for record in [&longest_id, &shortest_id, &biggest_key, &smallest_key] {
        let back = read(backend, &record.summary.key).expect("stored");
        assert!(same_fields(&back.summary, &record.summary));
        assert!(back.private_key.same_as(&record.private_key));
    }
    assert_eq!(sorted_keys(backend).len(), 4);
}

pub fn add_never_replaces(backend: &impl PasskeyBackend) {
    let first = passkey("example.com", 1, "alice");
    let key = first.summary.key.clone();
    assert_eq!(backend.add(&first).expect("add"), AddOutcome::Added);

    let other_key = PasskeyRecord {
        summary: summary("example.com", &id(1), "mallory"),
        private_key: private_key(2),
    };
    assert_eq!(
        backend.add(&other_key).expect("add the same key again"),
        AddOutcome::Exists
    );
    let back = read(backend, &key).expect("the stored passkey is still there");
    assert!(same_fields(&back.summary, &first.summary));
    assert!(back.private_key.same_as(&first.private_key));
    assert!(!back.private_key.same_as(&other_key.private_key));
    assert_eq!(sorted_keys(backend), vec![key]);
}

pub fn bulk_removal_through_the_store<B: PasskeyBackend>(backend: B) -> B {
    let store = PasskeyStore::new(backend);
    store
        .save(&passkey("example.com", 1, "alice"))
        .expect("save");
    store.save(&passkey("example.com", 2, "bob")).expect("save");
    store
        .save(&passkey("example.org", 3, "carol"))
        .expect("save");
    assert_eq!(store.all().expect("all").len(), 3);
    assert_eq!(store.for_rp("example.com").expect("for_rp").len(), 2);
    assert_eq!(store.for_rp("www.example.com").expect("for_rp").len(), 0);

    let verifier = FakeVerifier::new();
    assert_eq!(
        store
            .remove_for_rp("example.com", &verifier)
            .expect("remove"),
        2
    );
    let left = store.all().expect("all");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].key.rp_id, "example.org");
    assert_eq!(
        store
            .remove_for_rp("example.com", &verifier)
            .expect("again"),
        0
    );

    store
        .save(&passkey("example.net", 4, "dave"))
        .expect("save");
    assert_eq!(store.remove_all(&verifier).expect("remove all"), 2);
    assert!(store.all().expect("all").is_empty());
    assert_eq!(store.remove_all(&verifier).expect("remove all again"), 0);
    store.into_backend()
}
