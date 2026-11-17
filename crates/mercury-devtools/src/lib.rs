// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Development and CI tools for Mercury Browser, in Rust so that the repository needs no
//! scripting language (`AGENTS.md`). Nothing here depends on the browser engine.
//!
//! - [`serve`]: the loopback server for `tests/` (`serve-tests`, run through
//!   `scripts/serve-tests.sh`), which the media and dialog checks load their pages from.
//! - [`colors`] and [`png`]: the CI smoke test's check of a screenshot of
//!   `tests/pages/colors.html` (`check-colors-screenshot`).
//! - [`metadata`]: reads `cargo metadata` output for the macOS scripts when `jq` is missing
//!   (`package-manifest`).

pub mod colors;
pub mod httpdate;
pub mod metadata;
pub mod png;
pub mod serve;
