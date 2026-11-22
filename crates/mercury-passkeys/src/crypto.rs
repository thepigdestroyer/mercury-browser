// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The crypto backend of the built-in authenticator: ES256 only, with the operating system's
//! random number generator.
//!
//! passkey-rs' `RustCryptoBackend` does the real work (P-256 keys, ECDSA, SHA-256). Two things are
//! changed here, both decided in the lane brief:
//!
//! - **Only ES256 is offered.** Its keys come from `Generate::generate()`, which reads the
//!   operating system's random source; Ed25519 and ML-DSA keys and the credential ids of the stock
//!   backend come from `rand::rng()`, a CSPRNG seeded from the system but not the system RNG
//!   itself. A page that lists only other algorithms gets `UnsupportedAlgorithm`.
//! - **Every random byte (credential ids included) comes from `getrandom`.** [`OsRng`] implements
//!   passkey-rs' `RngBackend`, whose methods cannot return an error. [`probe_os_rng`] checks once,
//!   when the built-in authenticator is made, that the system RNG works; if it fails later anyway,
//!   [`OsRng`] aborts the process rather than ever return weak or zero bytes.

use std::ops::RangeInclusive;

use passkey::crypto::rng::RngBackend;
use passkey::crypto::rust_crypto::{RustCryptoBackend, RustCryptoSecretKey, RustCryptoSha2};
use passkey::crypto::{CryptoBackend, Error, iana};

use crate::PasskeyError;

/// Fills `buffer` from the operating system, or aborts the process.
///
/// A key or a credential id made from a failed generator could be predictable, and there is no
/// error path in `RngBackend` to report the failure through, so stopping is the only safe answer.
fn fill_or_abort(buffer: &mut [u8]) {
    if getrandom::fill(buffer).is_err() {
        std::process::abort();
    }
}

/// Checks that the operating system's random number generator answers.
///
/// The built-in authenticator calls this once when it is made, so that a broken system RNG is an
/// error the caller can show instead of a later abort.
pub fn probe_os_rng() -> Result<(), PasskeyError> {
    let mut probe = [0u8; 16];
    getrandom::fill(&mut probe).map_err(|_| PasskeyError::RandomUnavailable)
}

/// passkey-rs' `RngBackend` over the operating system's random number generator (`getrandom`).
///
/// If the system RNG fails, the process is aborted (see the module notes and [`probe_os_rng`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct OsRng;

impl RngBackend for OsRng {
    fn random_vec(len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        fill_or_abort(&mut bytes);
        bytes
    }

    fn random_array<const N: usize>() -> [u8; N] {
        let mut bytes = [0u8; N];
        fill_or_abort(&mut bytes);
        bytes
    }

    /// A uniform number in `range`, by rejection sampling so no value is more likely than another.
    /// An empty range (start after end) gives its start.
    fn from_range(range: RangeInclusive<u8>) -> u8 {
        let (start, end) = (*range.start(), *range.end());
        if start >= end {
            return start;
        }
        let span = u16::from(end - start) + 1;
        // The largest multiple of `span` that fits in 256 values: bytes at or above it are redrawn.
        let accepted = 256 - 256 % span;
        loop {
            let [byte] = Self::random_array::<1>();
            let byte = u16::from(byte);
            if byte < accepted {
                return start + u8::try_from(byte % span).unwrap_or(0);
            }
        }
    }
}

/// The crypto backend of the built-in authenticator: ES256 keys and hashing from passkey-rs'
/// RustCrypto backend, the operating system RNG for everything random, no other algorithm.
#[derive(Clone, Copy, Debug, Default)]
pub struct MercuryCrypto;

impl MercuryCrypto {
    /// The backend.
    pub fn new() -> Self {
        Self
    }
}

impl CryptoBackend for MercuryCrypto {
    type Rng = OsRng;
    type Sha256 = RustCryptoSha2;
    type SecretKey = RustCryptoSecretKey;

    fn enumerate_algorithms(&self) -> Vec<iana::Algorithm> {
        vec![iana::Algorithm::ES256]
    }

    fn generate_key(&self, algorithm: iana::Algorithm) -> Result<Self::SecretKey, Error> {
        match algorithm {
            iana::Algorithm::ES256 => RustCryptoBackend.generate_key(algorithm),
            _ => Err("Algorithm is unsupported".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use passkey::crypto::{PublicKeyT, SecretKeyT};

    use super::*;

    #[test]
    fn only_es256_is_offered() {
        let algorithms = MercuryCrypto::new().enumerate_algorithms();
        assert_eq!(algorithms, vec![iana::Algorithm::ES256]);
    }

    #[test]
    fn keys_are_made_for_es256_only() {
        let crypto = MercuryCrypto::new();
        let key = crypto
            .generate_key(iana::Algorithm::ES256)
            .expect("an ES256 key");
        assert_eq!(key.to_cose_key().alg, Some(iana::Algorithm::ES256.into()));
        for other in [
            iana::Algorithm::EdDSA,
            iana::Algorithm::Ed25519,
            iana::Algorithm::ESP256,
            iana::Algorithm::ML_DSA_44,
            iana::Algorithm::RS256,
        ] {
            assert!(
                crypto.generate_key(other).is_err(),
                "{other:?} must be refused"
            );
        }
    }

    #[test]
    fn two_keys_differ() {
        let crypto = MercuryCrypto::new();
        let first = crypto.generate_key(iana::Algorithm::ES256).expect("key");
        let second = crypto.generate_key(iana::Algorithm::ES256).expect("key");
        assert_ne!(
            first.public_key().to_cose_key(),
            second.public_key().to_cose_key()
        );
    }

    #[test]
    fn random_vectors_have_the_requested_length() {
        for length in [0usize, 1, 16, 32, 64, 1023] {
            assert_eq!(OsRng::random_vec(length).len(), length);
        }
    }

    #[test]
    fn random_vectors_are_not_constant() {
        // Not a statistical test of the generator (getrandom is the operating system's): it only
        // catches a vector that was never filled.
        let first = OsRng::random_vec(32);
        let second = OsRng::random_vec(32);
        assert_ne!(first, second);
        assert_ne!(first, vec![0u8; 32]);
        assert_ne!(OsRng::random_array::<32>(), [0u8; 32]);
    }

    #[test]
    fn a_range_is_respected_and_covered() {
        let mut seen = [false; 5];
        for _ in 0..2000 {
            let value = OsRng::from_range(16..=20);
            assert!((16..=20).contains(&value));
            seen[usize::from(value - 16)] = true;
        }
        assert!(seen.iter().all(|hit| *hit), "every value turns up");
    }

    #[test]
    fn ranges_at_the_edges_do_not_overflow() {
        assert_eq!(OsRng::from_range(7..=7), 7);
        assert_eq!(OsRng::from_range(RangeInclusive::new(9, 3)), 9);
        let _ = OsRng::from_range(0..=255);
        let value = OsRng::from_range(250..=255);
        assert!(value >= 250);
    }

    #[test]
    fn the_probe_succeeds_on_a_working_system() {
        assert_eq!(probe_os_rng(), Ok(()));
    }
}
