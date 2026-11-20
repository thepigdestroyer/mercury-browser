// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The whole decision table for a PDF offer (the PDF hand-off's decisions 3 to 8): every row,
//! in the order the rule applies them.

use mercury_downloads::{
    Attachment, Disposition, Facts, PDF_MARKER_WINDOW, PdfPreference, PdfRefusal, is_attachment,
    is_pdf_mime, looks_like_pdf, pdf_disposition,
};

const PREFERENCES: [PdfPreference; 3] = [
    PdfPreference::OpenInViewer,
    PdfPreference::OpenInSystemViewer,
    PdfPreference::Download,
];

/// A PDF over `https`, in the foreground tab, that a page started: the plainest offer.
fn facts() -> Facts<'static> {
    Facts {
        mime_type: "application/pdf",
        scheme: "https",
        user_started: false,
        foreground: true,
        posted: None,
        attachment: Attachment::Unknown,
        automated: false,
    }
}

fn viewer(read_in_place: bool) -> Disposition {
    Disposition::OpenInViewer { read_in_place }
}

fn decide(preference: PdfPreference, viewer_available: bool, facts: &Facts<'_>) -> Disposition {
    pdf_disposition(preference, viewer_available, facts)
}

// ── 1. What is a PDF ──────────────────────────────────────────────────────────────────────

#[test]
fn only_the_two_pdf_types_are_pdfs_whatever_their_parameters_or_case() {
    for mime in [
        "application/pdf",
        "application/x-pdf",
        "Application/PDF",
        "APPLICATION/X-PDF",
        " application/pdf ",
        "application/pdf; charset=binary",
        "application/pdf;q=1",
    ] {
        assert!(is_pdf_mime(mime), "{mime:?}");
    }
    for mime in [
        "",
        "pdf",
        "application/pdfx",
        "application/x-pdf-extra",
        "application/octet-stream",
        "text/html",
        "text/pdf",
        "image/pdf",
        "application/vnd.pdf",
        "application/pdf/extra",
        "; application/pdf",
    ] {
        assert!(!is_pdf_mime(mime), "{mime:?}");
    }
}

#[test]
fn anything_that_is_not_a_pdf_is_left_to_the_ordinary_rules_in_every_row() {
    for mime in ["text/html", "application/zip", "", "application/pdfx"] {
        for preference in PREFERENCES {
            for viewer_available in [false, true] {
                for user_started in [false, true] {
                    for attachment in [Attachment::Unknown, Attachment::Yes, Attachment::No] {
                        let offer = Facts {
                            mime_type: mime,
                            user_started,
                            attachment,
                            ..facts()
                        };
                        assert_eq!(
                            decide(preference, viewer_available, &offer),
                            Disposition::NotPdf,
                            "{mime:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn both_pdf_types_are_treated_alike() {
    for mime in ["application/pdf", "application/x-pdf"] {
        let offer = Facts {
            mime_type: mime,
            ..facts()
        };
        assert_eq!(
            decide(PdfPreference::OpenInViewer, true, &offer),
            viewer(false)
        );
    }
}

// ── 2. An attachment is a download ────────────────────────────────────────────────────────

#[test]
fn an_attachment_is_a_download_whatever_the_setting_and_the_facts() {
    for preference in PREFERENCES {
        for viewer_available in [false, true] {
            for user_started in [false, true] {
                for foreground in [false, true] {
                    for scheme in ["https", "file", "blob"] {
                        for automated in [false, true] {
                            let offer = Facts {
                                attachment: Attachment::Yes,
                                user_started,
                                foreground,
                                scheme,
                                automated,
                                posted: Some(true),
                                ..facts()
                            };
                            assert_eq!(
                                decide(preference, viewer_available, &offer),
                                Disposition::Download
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn an_inline_or_unknown_header_does_not_change_the_answer() {
    for attachment in [Attachment::Unknown, Attachment::No] {
        let offer = Facts {
            attachment,
            ..facts()
        };
        assert_eq!(
            decide(PdfPreference::OpenInViewer, true, &offer),
            viewer(false)
        );
    }
}

// ── 3. The setting Download ───────────────────────────────────────────────────────────────

#[test]
fn the_download_setting_always_downloads() {
    for viewer_available in [false, true] {
        for user_started in [false, true] {
            for foreground in [false, true] {
                for automated in [false, true] {
                    let offer = Facts {
                        user_started,
                        foreground,
                        automated,
                        ..facts()
                    };
                    assert_eq!(
                        decide(PdfPreference::Download, viewer_available, &offer),
                        Disposition::Download
                    );
                }
            }
        }
    }
}

// ── 4 and 5. The system's PDF app ─────────────────────────────────────────────────────────

#[test]
fn the_system_viewer_opens_the_file_only_for_a_navigation_the_user_started() {
    for viewer_available in [false, true] {
        for foreground in [false, true] {
            let user = Facts {
                user_started: true,
                foreground,
                ..facts()
            };
            assert_eq!(
                decide(PdfPreference::OpenInSystemViewer, viewer_available, &user),
                Disposition::DownloadThenOpen,
                "foreground {foreground}"
            );
            let page = Facts {
                user_started: false,
                foreground,
                ..facts()
            };
            // A page cannot launch an application: it is an ordinary download.
            assert_eq!(
                decide(PdfPreference::OpenInSystemViewer, viewer_available, &page),
                Disposition::Download,
                "foreground {foreground}"
            );
        }
    }
}

#[test]
fn an_automated_run_never_opens_the_system_viewer_it_only_notes_that_it_would() {
    for viewer_available in [false, true] {
        let user = Facts {
            user_started: true,
            automated: true,
            ..facts()
        };
        assert_eq!(
            decide(PdfPreference::OpenInSystemViewer, viewer_available, &user),
            Disposition::DownloadWouldOpen
        );
        let page = Facts {
            user_started: false,
            automated: true,
            ..facts()
        };
        assert_eq!(
            decide(PdfPreference::OpenInSystemViewer, viewer_available, &page),
            Disposition::Download
        );
    }
}

#[test]
fn without_a_viewer_the_default_setting_behaves_as_the_system_viewer() {
    let typed = Facts {
        user_started: true,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, false, &typed),
        Disposition::DownloadThenOpen
    );
    assert_eq!(
        decide(PdfPreference::OpenInViewer, false, &facts()),
        Disposition::Download
    );
    let automated = Facts {
        user_started: true,
        automated: true,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, false, &automated),
        Disposition::DownloadWouldOpen
    );
    // The same offers with a viewer go to it.
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &typed),
        viewer(false)
    );
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &facts()),
        viewer(false)
    );
}

// ── 6. The viewer ─────────────────────────────────────────────────────────────────────────

#[test]
fn any_navigation_of_the_foreground_tab_opens_the_viewer() {
    for user_started in [false, true] {
        let offer = Facts {
            user_started,
            ..facts()
        };
        assert_eq!(
            decide(PdfPreference::OpenInViewer, true, &offer),
            viewer(false)
        );
    }
}

#[test]
fn a_background_tab_shows_a_pdf_only_when_the_user_started_it() {
    let page = Facts {
        foreground: false,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &page),
        Disposition::Refuse(PdfRefusal::Background)
    );
    let ctrl_click = Facts {
        foreground: false,
        user_started: true,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &ctrl_click),
        viewer(false)
    );
}

#[test]
fn web_addresses_are_fetched_and_other_schemes_are_refused() {
    for scheme in ["http", "https"] {
        let offer = Facts { scheme, ..facts() };
        assert_eq!(
            decide(PdfPreference::OpenInViewer, true, &offer),
            viewer(false),
            "{scheme}"
        );
    }
    for scheme in [
        "blob",
        "data",
        "ftp",
        "about",
        "javascript",
        "view-source",
        "",
    ] {
        for user_started in [false, true] {
            let offer = Facts {
                scheme,
                user_started,
                ..facts()
            };
            assert_eq!(
                decide(PdfPreference::OpenInViewer, true, &offer),
                Disposition::Refuse(PdfRefusal::Unsupported(scheme.to_owned())),
                "{scheme:?}"
            );
        }
    }
}

#[test]
fn a_file_address_opens_in_place_only_when_the_user_started_it() {
    let typed = Facts {
        scheme: "file",
        user_started: true,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &typed),
        viewer(true)
    );
    // A page cannot send a tab to a PDF on this computer.
    let page = Facts {
        scheme: "file",
        user_started: false,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &page),
        Disposition::Refuse(PdfRefusal::FileFromPage)
    );
    // In the background the typed address is still the user's, and the page's is still refused
    // for being in the background.
    let background_page = Facts {
        foreground: false,
        ..page.clone()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &background_page),
        Disposition::Refuse(PdfRefusal::Background)
    );
    let background_typed = Facts {
        foreground: false,
        ..typed
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &background_typed),
        viewer(true)
    );
}

#[test]
fn a_forms_post_is_refused_and_an_unknown_method_is_not() {
    let posted = Facts {
        posted: Some(true),
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &posted),
        Disposition::Refuse(PdfRefusal::Posted)
    );
    for known_get in [Some(false), None] {
        let offer = Facts {
            posted: known_get,
            ..facts()
        };
        assert_eq!(
            decide(PdfPreference::OpenInViewer, true, &offer),
            viewer(false)
        );
    }
}

#[test]
fn the_refusals_apply_in_the_order_background_scheme_post() {
    let everything_wrong = Facts {
        foreground: false,
        user_started: false,
        scheme: "blob",
        posted: Some(true),
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &everything_wrong),
        Disposition::Refuse(PdfRefusal::Background)
    );
    let scheme_and_post = Facts {
        foreground: true,
        ..everything_wrong.clone()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &scheme_and_post),
        Disposition::Refuse(PdfRefusal::Unsupported("blob".to_owned()))
    );
    let post_only = Facts {
        scheme: "https",
        ..scheme_and_post
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &post_only),
        Disposition::Refuse(PdfRefusal::Posted)
    );
}

#[test]
fn an_automated_run_still_uses_the_viewer() {
    let offer = Facts {
        automated: true,
        user_started: true,
        ..facts()
    };
    assert_eq!(
        decide(PdfPreference::OpenInViewer, true, &offer),
        viewer(false)
    );
}

// ── The Content-Disposition header ────────────────────────────────────────────────────────

#[test]
fn only_a_disposition_type_other_than_inline_asks_for_the_file_to_be_saved() {
    assert!(!is_attachment(None));
    for header in [
        "",
        "   ",
        "inline",
        "INLINE",
        "Inline; filename=\"a.pdf\"",
        "inline;filename*=UTF-8''a.pdf",
        // No type at all: the parser reads the name, and the response is shown.
        "filename=\"a.pdf\"",
        ";",
    ] {
        assert!(!is_attachment(Some(header)), "{header:?}");
    }
    for header in [
        "attachment",
        "Attachment",
        "ATTACHMENT; filename=\"report.pdf\"",
        " attachment ; filename=a.pdf",
        // RFC 6266 §4.2: an unknown type is handled as an attachment.
        "form-data; name=\"x\"",
        "download",
    ] {
        assert!(is_attachment(Some(header)), "{header:?}");
    }
}

// ── The %PDF- marker ──────────────────────────────────────────────────────────────────────

#[test]
fn the_marker_must_be_within_the_first_kibibyte() {
    assert!(looks_like_pdf(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"));
    let junk = |count: usize| vec![b' '; count];
    let with_marker_at = |offset: usize| {
        let mut bytes = junk(offset);
        bytes.extend_from_slice(b"%PDF-1.4 and the rest of the file");
        bytes
    };
    assert!(looks_like_pdf(&with_marker_at(100)));
    // The marker's last byte is the last byte of the window.
    assert!(looks_like_pdf(&with_marker_at(PDF_MARKER_WINDOW - 5)));
    // One byte further, it straddles the end of the window.
    assert!(!looks_like_pdf(&with_marker_at(PDF_MARKER_WINDOW - 4)));
    assert!(!looks_like_pdf(&with_marker_at(PDF_MARKER_WINDOW)));
    assert!(!looks_like_pdf(&with_marker_at(5000)));
}

#[test]
fn things_that_are_not_a_pdf_are_not_taken_for_one() {
    for bytes in [
        &b""[..],
        b"%PDF",
        b"%PDF ",
        b"%pdf-1.7",
        b"%PDF1.7",
        b"<!doctype html><title>%PD F-</title>",
        b"PK\x03\x04 a zip file",
        &[0_u8; 2000],
    ] {
        assert!(!looks_like_pdf(bytes), "{bytes:?}");
    }
}

#[test]
fn only_the_marker_window_is_looked_at_so_a_huge_slice_is_cheap() {
    let mut bytes = vec![0_u8; 10_000_000];
    bytes.extend_from_slice(b"%PDF-1.7");
    assert!(!looks_like_pdf(&bytes));
}

#[test]
fn the_preference_default_is_the_viewer() {
    assert_eq!(PdfPreference::default(), PdfPreference::OpenInViewer);
}
