// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: runs the unit tests inside Mercury's two lib files, which `[lib] test = false` never compiles.

//! The unit tests of `mercury_log.rs` (cookie and credential redaction, EC13) and `mercury_svg.rs`
//! (the SVG depth guard, D8). The library has `test = false`, so the tests inside those files
//! would run in no gate; this module compiles them into the `main` test target instead.
//! `include_str!` in them is relative to the real files, so their source-scan guards read the
//! crate's `cookie_storage.rs`, `cookie.rs` and `http_loader.rs`.

#[path = "../mercury_log.rs"]
mod mercury_log;
#[path = "../mercury_svg.rs"]
mod mercury_svg;
