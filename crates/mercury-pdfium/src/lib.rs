// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDFium backend of Mercury Browser's PDF viewer and the helper process's entry point.
//!
//! - [`PdfiumBackend`] implements `mercury_pdf::backend::PdfBackend` with `pdfium-render`: one
//!   document opened by path, tiles, text, find, links, outline. PDFium is **not linked**; it is
//!   loaded at run time from a prebuilt dynamic library (`scripts/macos/pdfium.sh` fetches it,
//!   without V8 and without XFA), so this crate builds and its non-PDFium tests run without it.
//! - [`locate_library`] says where that library is (environment, next to the executable, the
//!   macOS cache).
//! - [`run_worker_if_requested`] is called first thing by `main`: with `--pdf-worker` the process
//!   becomes the helper that runs the request loop on standard input and output.
//!
//! The helper's process hygiene (no terminal, no core dumps, signals ignored, protocol on its own
//! descriptor) is the only `unsafe` code of the viewer; it is in `worker`.

#![deny(unsafe_code)]

mod backend;
mod geometry;
mod library;
mod worker;

pub use backend::{PdfiumBackend, char_range};
pub use geometry::PageGeometry;
pub use library::{
    LIBRARY_FILE, LIBRARY_VARIABLE, PDFIUM_RELEASE, Search, cache_arch, locate_library,
};
pub use worker::{WORKER_FLAG, run_worker_if_requested, worker_requested};
