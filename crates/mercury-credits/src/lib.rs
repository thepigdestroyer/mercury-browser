// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `mercury-credits` writes `credits.html`, one page with the licence text, copyright line,
//! Apache NOTICE and MPL source pointer of every crate built into the app, and checks that every
//! crate can be credited. It reads `cargo tree` (which crates), `cargo metadata` (what they are),
//! `deny.toml` (which licences Mercury may use), `third_party/vendor.toml` and
//! `crates/mercury-credits/credits.toml`; it never uses the network itself.
//!
//! See `docs/developer-guide/licence-compliance.md`.

pub mod app;
pub mod cargo;
pub mod classify;
pub mod cli;
pub mod collect;
pub mod config;
pub mod discover;
pub mod error;
pub mod page;
pub mod spdx;
