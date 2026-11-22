// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Test doubles for the engine and UI lanes and for this crate's own tests: a scripted user verifier and a fake security key. Compiled for this crate's tests and with the `testing` feature.

mod security_key;
mod verifier;

pub use security_key::*;
pub use verifier::*;
