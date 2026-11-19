// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDF helper as a program of its own, for this crate's tests and for measuring it: the same
//! code as `mercury-browser --pdf-worker` (`run_worker_if_requested`), without the browser.
//!
//! `mercury-pdf-worker --pdf-worker` is started by a `PdfClient`, never by hand. The browser does
//! not use this program; it re-runs its own executable.

fn main() {
    if mercury_pdfium::run_worker_if_requested() {
        return;
    }
    eprintln!(
        "mercury-pdf-worker is started by the browser's PDF client with {}; it is not for people.",
        mercury_pdfium::WORKER_FLAG
    );
    std::process::exit(2);
}
