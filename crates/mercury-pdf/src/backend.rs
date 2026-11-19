// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What a PDF engine must do for the viewer. [`crate::fake::FakeBackend`] implements it for
//! tests; `mercury_pdfium::PdfiumBackend` implements it with PDFium.
//!
//! A backend runs in the PDF helper process on **one thread** (PDFium is not thread-safe), so
//! nothing here is `Send` or `Sync`, and a call may take as long as the file needs: the helper's
//! request loop (`crate::serve`) decides what runs next and cancels what no longer matters.
//! A backend holds at most one document; the browser starts one helper process per document.
//!
//! The request loop checks [`crate::types::TileRequest::validate`] and the page index before it
//! calls the backend, but a backend must still not trust its caller: a bad index is
//! [`PdfError::InvalidRequest`], never a panic. Everything a backend returns is clamped to
//! [`crate::types::limits`].

use std::path::Path;

use crate::error::PdfError;
use crate::types::{
    DocumentInfo, FindOptions, Link, OutlineItem, PageText, SearchHit, Tile, TileRequest,
};

pub trait PdfBackend {
    /// Opens the file, replacing any document that is open. Reads it from disk as it needs it
    /// (the file may be hundreds of megabytes, so it is not read whole into memory unless the
    /// engine insists). A file of more than [`crate::types::limits::MAX_FILE_BYTES`] or
    /// [`crate::types::limits::MAX_PAGES`] pages is `TooLarge`; an encrypted one is
    /// `PasswordRequired` without a password and `PasswordIncorrect` with a wrong one. The
    /// password is never logged.
    fn open(&mut self, path: &Path, password: Option<&str>) -> Result<DocumentInfo, PdfError>;

    /// Renders one tile with form fields drawn (read-only) and annotations on, on an opaque
    /// white page.
    fn render(&mut self, request: &TileRequest) -> Result<Tile, PdfError>;

    /// The page's text in reading order, at most [`crate::types::limits::MAX_TEXT_CHARS`].
    fn page_text(&mut self, page: u32) -> Result<PageText, PdfError>;

    /// Every match of `query` on one page, at most
    /// [`crate::types::limits::MAX_HITS_PER_PAGE`], in reading order. An empty query matches
    /// nothing.
    fn find_in_page(
        &mut self,
        page: u32,
        query: &str,
        options: FindOptions,
    ) -> Result<Vec<SearchHit>, PdfError>;

    /// The clickable links of one page.
    fn links(&mut self, page: u32) -> Result<Vec<Link>, PdfError>;

    /// The document outline, within [`crate::types::limits::MAX_OUTLINE_ITEMS`] entries and
    /// [`crate::types::limits::MAX_OUTLINE_DEPTH`] levels; empty when there is none.
    fn outline(&mut self) -> Result<Vec<OutlineItem>, PdfError>;

    /// Closes the document and frees what it held. Safe to call with nothing open.
    fn close(&mut self);
}
