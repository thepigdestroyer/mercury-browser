// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The download engine of Mercury Browser: naming, sanitising, streaming to disk, progress,
//! cancel and resume.
//!
//! Servo has no download support, so the browser fetches files itself. Every name, path and URL
//! handled here comes from a web server, which makes this crate security-sensitive: a downloaded
//! file must never land outside the folder chosen for it, never replace an existing file, and
//! never carry a name that lies about what it is.

mod content_disposition;
pub mod disposition;
mod error;
mod manager;
pub mod naming;
mod quarantine;
mod transfer;

pub use disposition::{
    Attachment, Disposition, Facts, PDF_MARKER_WINDOW, PdfPreference, PdfRefusal, is_attachment,
    is_pdf_mime, looks_like_pdf, pdf_disposition,
};
pub use manager::{
    DownloadEvent, DownloadId, DownloadManager, DownloadRequest, ResumeFrom, Timeouts,
};
pub use naming::{file_name_for, format_bytes, sanitize_file_name, unique_path};
pub use transfer::remove_partial_file;
