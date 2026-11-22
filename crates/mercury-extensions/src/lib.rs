// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Browser extensions without the engine (EE32, EE33): packages, signatures, the manifest, updates, and
//! the permission model. A package is untrusted input; see `docs/developer-guide/extensions.md`.

#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod archive;
pub mod crx;
pub mod id;
pub mod install;
pub mod manifest;
pub mod match_pattern;
pub mod permissions;
pub mod roots;
pub mod update;
pub mod xpi;
