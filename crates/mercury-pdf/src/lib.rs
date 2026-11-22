// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Mercury Browser's PDF viewer, without PDFium and without the engine or the interface toolkit.
//!
//! PDFium (a C++ library, unsafe to run on untrusted files) never runs in the browser's own
//! process. The browser starts a helper process, the same executable with `--pdf-worker`, one per
//! open document; the two talk over the helper's standard input and output ([`protocol`]).
//!
//! - [`types`], [`error`] and [`backend`]: the shared vocabulary and the trait a PDF engine
//!   implements. Frozen by the lane lead; changes go through the integrator.
//! - [`protocol`], [`serve`], [`client`], [`fake`] and [`testing`]: the process side.
//! - [`layout`], [`cache`], [`selection`], [`find`] and [`viewer`]: the view side, pure logic
//!   with no I/O and no clock, tested without a window.
//!
//! The crate forbids `unsafe`; the only unsafe code of the viewer is in `mercury-pdfium`.

#![forbid(unsafe_code)]

pub mod backend;
pub mod cache;
mod clean;
pub mod client;
pub mod error;
pub mod fake;
pub mod find;
pub mod layout;
pub mod protocol;
pub mod selection;
pub mod serve;
pub mod testing;
pub mod types;
pub mod viewer;
