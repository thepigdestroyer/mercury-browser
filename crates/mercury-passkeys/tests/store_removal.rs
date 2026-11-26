// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Removing stored passkeys needs the user to be verified. A refused removal removes nothing and
//! returns `NotAllowed`; an allowed one removes as before.

mod common;

use common::contract::{id, key, passkey};
use mercury_passkeys::testing::{FakeAnswer, FakeVerifier};
use mercury_passkeys::{MemoryBackend, PasskeyError, PasskeyStore};

fn store_with_two_sites() -> PasskeyStore<MemoryBackend> {
    let store = PasskeyStore::new(MemoryBackend::new());
    store
        .save(&passkey("example.com", 1, "alice"))
        .expect("save");
    store.save(&passkey("example.org", 2, "bob")).expect("save");
    store
}

#[test]
fn a_refused_single_removal_removes_nothing_and_errors() {
    let store = PasskeyStore::new(MemoryBackend::new());
    store
        .save(&passkey("example.com", 1, "alice"))
        .expect("save");
    let verifier = FakeVerifier::new();
    verifier.set_default_answer(FakeAnswer::Decline);

    let result = store.remove(&key("example.com", &id(1)), &verifier);
    assert!(matches!(result, Err(PasskeyError::NotAllowed)));
    assert_eq!(store.all().expect("all").len(), 1, "nothing was removed");
    assert_eq!(verifier.requests().len(), 1, "the user was asked");
}

#[test]
fn an_allowed_single_removal_removes_the_passkey() {
    let store = PasskeyStore::new(MemoryBackend::new());
    store
        .save(&passkey("example.com", 1, "alice"))
        .expect("save");
    let verifier = FakeVerifier::new();

    assert!(
        store
            .remove(&key("example.com", &id(1)), &verifier)
            .expect("removed")
    );
    assert!(store.all().expect("all").is_empty());
    assert_eq!(verifier.requests().len(), 1, "the user was asked");
}

#[test]
fn a_refused_removal_for_one_site_removes_nothing_and_errors() {
    let store = store_with_two_sites();
    let verifier = FakeVerifier::new();
    store
        .save(&passkey("example.com", 3, "carol"))
        .expect("save");
    verifier.set_default_answer(FakeAnswer::Decline);

    let result = store.remove_for_rp("example.com", &verifier);
    assert!(matches!(result, Err(PasskeyError::NotAllowed)));
    assert_eq!(store.for_rp("example.com").expect("for_rp").len(), 2);
}

#[test]
fn a_refused_removal_of_everything_removes_nothing_and_errors() {
    let store = store_with_two_sites();
    let verifier = FakeVerifier::new();
    verifier.set_default_answer(FakeAnswer::Decline);

    let result = store.remove_all(&verifier);
    assert!(matches!(result, Err(PasskeyError::NotAllowed)));
    assert_eq!(store.all().expect("all").len(), 2, "nothing was removed");
    assert_eq!(verifier.requests().len(), 1, "the user was asked");
}

#[test]
fn an_allowed_removal_of_everything_removes_every_passkey() {
    let store = PasskeyStore::new(MemoryBackend::new());
    store
        .save(&passkey("example.com", 1, "alice"))
        .expect("save");
    store.save(&passkey("example.org", 2, "bob")).expect("save");
    let verifier = FakeVerifier::new();

    assert_eq!(store.remove_all(&verifier).expect("removed"), 2);
    assert!(store.all().expect("all").is_empty());
}
