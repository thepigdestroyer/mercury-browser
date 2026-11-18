// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Carries Mercury's patches to vendored crates (Servo's above all) in a form
//! that survives upgrades: each crate in `third_party/<name>/` is the archive
//! published on crates.io plus one exported patch,
//! `third_party/patches/<name>.patch`, recorded in `third_party/vendor.toml`.

pub mod archive;
pub mod commands;
pub mod error;
pub mod lock;
pub mod manifest;
pub mod patch;
pub mod registry;
pub mod tree;

#[cfg(test)]
mod tests;
