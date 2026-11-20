// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where a page the engine could not show goes when it is a PDF file: Mercury's own viewer, the
//! system's PDF app, or an ordinary download.
//!
//! A pure rule, so that the whole table can be tested: [`pdf_disposition`] takes what the user
//! chose, whether the viewer exists, and the facts of one offer, and says what to do. It does no
//! I/O and knows nothing of tabs, settings or the engine. The browser maps its own setting to
//! [`PdfPreference`] (this crate does not depend on the settings crate) and carries the answer
//! out.
//!
//! The decisions, in the order the function applies them:
//!
//! 1. Only `application/pdf` and `application/x-pdf` are PDFs here (parameters and case are
//!    ignored); anything else is [`Disposition::NotPdf`] and the ordinary rules apply.
//! 2. A response sent as an attachment is a download, whatever the setting. The header is not
//!    known when the offer arrives (the engine does not report it), so the first answer for an
//!    unknown header may be "viewer"; the fetch learns it (`DownloadEvent::Started::attachment`)
//!    and asks again with [`Attachment::Yes`].
//! 3. The setting `Download` is a download.
//! 4. `OpenInViewer` without a viewer (no PDFium library, say) is `OpenInSystemViewer`.
//! 5. `OpenInSystemViewer` is a download followed by opening the file with the system's PDF
//!    app, but only for a navigation the user started, and never in an automated run (which
//!    downloads and notes that it would have opened it). Anything else is an ordinary download:
//!    a page cannot make Mercury launch an application.
//! 6. For the viewer: a tab in the background shows nothing unless the user started the
//!    navigation (a Ctrl-click), and then the viewer opens when the tab is shown; only `http`
//!    and `https` addresses are fetched, plus `file:` when the user started the navigation (read
//!    where it is, never copied); `blob:`, `data:` and everything else are refused; a form's
//!    POST is refused because it cannot be sent again.
//!
//! "A download" here means the browser's ordinary download rules apply (they have their own
//! checks and may ask on the tab's bar): this module only says "not the viewer".

/// What the user chose for PDF files (the browser's own setting, mapped).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PdfPreference {
    /// Show it in Mercury's viewer, in the tab.
    #[default]
    OpenInViewer,
    /// Download it, then open it with the system's PDF app.
    OpenInSystemViewer,
    /// Download it.
    Download,
}

/// What the response's `Content-Disposition` said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attachment {
    /// Not seen yet: the offer arrived before the response was read.
    Unknown,
    /// The disposition type is `attachment` (or any type but `inline`).
    Yes,
    /// There was no header, or it said `inline`.
    No,
}

/// What is known about one PDF offer.
#[derive(Clone, Debug)]
pub struct Facts<'a> {
    /// The response's `Content-Type`, as the engine reported it.
    pub mime_type: &'a str,
    /// The address's scheme, lower case: `https`, `file`, `blob`...
    pub scheme: &'a str,
    /// The user started this navigation (a click, a typed address), as downloads decide it.
    pub user_started: bool,
    /// The tab is the one the user is looking at.
    pub foreground: bool,
    /// `Some(true)`: a form's POST; `None`: not known.
    pub posted: Option<bool>,
    /// The `Content-Disposition`, once it is known.
    pub attachment: Attachment,
    /// A `--screenshot` or `--background` run.
    pub automated: bool,
}

/// Why a PDF is not shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfRefusal {
    /// The tab is in the background and the user did not start the navigation.
    Background,
    /// The address's scheme cannot be fetched (`blob:`, `data:`, `ftp:`...).
    Unsupported(String),
    /// A page sent the tab to a PDF on this computer (`file:`); only the user can.
    FileFromPage,
    /// The page came from a form's POST, which cannot be sent again.
    Posted,
}

/// What to do with the PDF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Not a PDF: the ordinary download rules apply.
    NotPdf,
    /// Show it in the viewer. `read_in_place`: a `file:` address, read where it is (the viewer
    /// must neither copy nor delete the file); otherwise it is fetched into a private cache file.
    OpenInViewer {
        /// The file is read where it is.
        read_in_place: bool,
    },
    /// An ordinary download; when it has finished, open the file with the system's PDF app.
    DownloadThenOpen,
    /// An ordinary download in an automated run: it is only logged that the file would have been
    /// opened.
    DownloadWouldOpen,
    /// An ordinary download, as for any other file.
    Download,
    /// Nothing is fetched; the user hears why when they started the navigation.
    Refuse(PdfRefusal),
}

/// Whether a `Content-Type` names a PDF: `application/pdf` or `application/x-pdf`, ignoring
/// parameters, case and surrounding white space.
#[must_use]
pub fn is_pdf_mime(mime_type: &str) -> bool {
    let essence = mime_type.split(';').next().unwrap_or("").trim();
    essence.eq_ignore_ascii_case("application/pdf")
        || essence.eq_ignore_ascii_case("application/x-pdf")
}

/// Whether a `Content-Disposition` header asks for the file to be saved: its disposition type is
/// anything but `inline` (RFC 6266 §4.2: an unknown type is handled as `attachment`). No header,
/// an empty one, and one that starts straight with a parameter (no type) are not attachments.
#[must_use]
pub fn is_attachment(header: Option<&str>) -> bool {
    let Some(header) = header else {
        return false;
    };
    let first = header.split(';').next().unwrap_or("").trim();
    if first.is_empty() || first.contains('=') {
        return false;
    }
    !first.eq_ignore_ascii_case("inline")
}

/// How many leading bytes may hold the `%PDF-` marker (the PDF specification allows some junk
/// before it; PDFium reads up to 1,024).
pub const PDF_MARKER_WINDOW: usize = 1024;

/// Whether `head` (the first bytes of a file) carries the `%PDF-` marker within its first
/// [`PDF_MARKER_WINDOW`] bytes. Only that much is looked at.
#[must_use]
pub fn looks_like_pdf(head: &[u8]) -> bool {
    let window = &head[..head.len().min(PDF_MARKER_WINDOW)];
    window.windows(5).any(|bytes| bytes == b"%PDF-")
}

/// The decision table; see the module documentation for the order.
#[must_use]
pub fn pdf_disposition(
    preference: PdfPreference,
    viewer_available: bool,
    facts: &Facts<'_>,
) -> Disposition {
    if !is_pdf_mime(facts.mime_type) {
        return Disposition::NotPdf;
    }
    if facts.attachment == Attachment::Yes || preference == PdfPreference::Download {
        return Disposition::Download;
    }
    let effective = match preference {
        PdfPreference::OpenInViewer if !viewer_available => PdfPreference::OpenInSystemViewer,
        other => other,
    };
    if effective == PdfPreference::OpenInSystemViewer {
        return match (facts.user_started, facts.automated) {
            (false, _) => Disposition::Download,
            (true, true) => Disposition::DownloadWouldOpen,
            (true, false) => Disposition::DownloadThenOpen,
        };
    }

    // The viewer.
    if !facts.foreground && !facts.user_started {
        return Disposition::Refuse(PdfRefusal::Background);
    }
    let in_place = match facts.scheme {
        "http" | "https" => false,
        "file" if facts.user_started => true,
        "file" => return Disposition::Refuse(PdfRefusal::FileFromPage),
        other => return Disposition::Refuse(PdfRefusal::Unsupported(other.to_owned())),
    };
    if facts.posted == Some(true) {
        return Disposition::Refuse(PdfRefusal::Posted);
    }
    Disposition::OpenInViewer {
        read_in_place: in_place,
    }
}
