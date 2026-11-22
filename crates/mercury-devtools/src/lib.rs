// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Development and CI tools for Mercury Browser, in Rust so that the repository needs no
//! scripting language (`AGENTS.md`). Nothing here depends on the browser engine.
//!
//! - [`serve`]: the loopback server for `tests/` (`serve-tests`, run through
//!   `scripts/serve-tests.sh`), which the media and dialog checks load their pages from.
//! - [`colors`] and [`png`]: the CI smoke test's check of a screenshot of
//!   `tests/pages/colors.html` (`check-colors-screenshot`).
//! - [`metadata`]: reads `cargo metadata` output: for the macOS scripts when `jq` is missing
//!   (`package-manifest`), and the engine-free members the fast CI jobs check (`light-crates`,
//!   run through `scripts/ci/light-crates.sh`).
//! - [`github_env`]: the variables `scripts/ci/github-env.sh` passes to later GitHub Actions
//!   steps (`github-env`).
//! - [`pixel_diff`]: compares two folders of PNG pictures pixel by pixel (`pixel-diff`), the gate
//!   that a change to the interface draws what it drew before.

pub mod colors;
pub mod github_env;
pub mod metadata;
pub mod pixel_diff;
pub mod png;
pub mod serve;
