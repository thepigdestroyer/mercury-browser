// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDFium backend of Mercury Browser's PDF viewer and the helper process's entry point.
//!
//! (Placeholder: filled by lane item pdf-process. It will hold `PdfiumBackend`, which implements
//! `mercury_pdf::backend::PdfBackend` with `pdfium-render`, `locate_library`, which finds the
//! PDFium dynamic library, and `run_worker_if_requested`, which `main` calls first.)

// Keep both dependencies named until the code that uses them lands; delete these two lines then.
use mercury_pdf as _;
use pdfium_render as _;
