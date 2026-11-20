// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the PDF viewer's fetch relies on from the download engine, against the loopback test
//! server: the same request as a download (cookies, referrer, user agent), the response's
//! `Content-Disposition` on `Started`, redirects, a body that is not a PDF, and a fetch that a
//! newer offer cancels. The viewer's fetch is an ordinary transfer into a private folder; what
//! turns it into a viewer or a download is the browser's decision on these events.
//!
//! The fetch is also bounded (`DownloadRequest::max_bytes`): a server that says its PDF is too
//! big, or that streams one for ever, ends in a `Failed` event with no file left behind.

mod support;

use mercury_downloads::{
    Attachment, DownloadEvent, Facts, PdfPreference, is_pdf_mime, looks_like_pdf, pdf_disposition,
};
use support::{
    PDF_BODY, TempDir, TestServer, events_until_done, manager, request, terminal, wait_started,
};

fn started_attachment(event: &DownloadEvent) -> bool {
    match event {
        DownloadEvent::Started { attachment, .. } => *attachment,
        other => panic!("expected Started, got {other:?}"),
    }
}

fn started_mime(event: &DownloadEvent) -> Option<String> {
    match event {
        DownloadEvent::Started { mime_type, .. } => mime_type.clone(),
        other => panic!("expected Started, got {other:?}"),
    }
}

fn head_of(path: &std::path::Path) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("read the fetched file");
    bytes[..bytes.len().min(1024)].to_vec()
}

fn finished(events: &[DownloadEvent]) -> std::path::PathBuf {
    match terminal(events) {
        DownloadEvent::Finished { target_path, .. } => target_path.clone(),
        other => panic!("expected Finished, got {other:?}"),
    }
}

#[test]
fn an_inline_pdf_is_fetched_whole_and_is_not_an_attachment() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-inline");
    let (manager, events) = manager();
    let id = manager.start(request(server.url("/doc.pdf"), &dir.path));
    let events = events_until_done(&events, id);

    assert_eq!(started_mime(&events[0]).as_deref(), Some("application/pdf"));
    assert!(!started_attachment(&events[0]));
    let path = finished(&events);
    assert_eq!(std::fs::read(&path).expect("the file"), PDF_BODY);
    assert!(looks_like_pdf(&head_of(&path)));
    assert_eq!(TempDir::list(&dir.path), vec!["doc.pdf".to_owned()]);
}

#[test]
fn an_explicit_inline_disposition_is_not_an_attachment_either() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-explicit-inline");
    let (manager, events) = manager();
    let id = manager.start(request(server.url("/inline.pdf"), &dir.path));
    let events = events_until_done(&events, id);
    assert!(!started_attachment(&events[0]));
    // The name the server proposed is used, as for a download.
    assert_eq!(
        finished(&events).file_name().and_then(|name| name.to_str()),
        Some("inline.pdf")
    );
}

#[test]
fn an_attachment_pdf_says_so_and_the_second_request_is_an_ordinary_download() {
    let server = TestServer::start();
    let cache = TempDir::new("pdf-attachment-cache");
    let downloads = TempDir::new("pdf-attachment-downloads");
    let (manager, events) = manager();

    // The viewer's fetch: the response says "attachment", so the browser drops this copy ...
    let view = manager.start(request(server.url("/attachment.pdf"), &cache.path));
    let started = wait_started(&events, view);
    assert!(started_attachment(&started));
    manager.cancel(view);
    let view_events = events_until_done(&events, view);
    assert!(matches!(
        terminal(&view_events),
        DownloadEvent::Cancelled { .. } | DownloadEvent::Finished { .. }
    ));
    // ... removes whatever was kept ...
    if matches!(terminal(&view_events), DownloadEvent::Finished { .. }) {
        // The body is tiny: the fetch may have finished before the cancel arrived. The browser
        // deletes the cache file then; the test does the same.
        std::fs::remove_file(finished(&view_events)).expect("delete the cache copy");
    }
    assert_eq!(TempDir::list(&cache.path), Vec::<String>::new());

    // ... and starts the download, which is a second request for the same address.
    let download = manager.start(request(server.url("/attachment.pdf"), &downloads.path));
    let download_events = events_until_done(&events, download);
    assert!(started_attachment(&download_events[0]));
    let saved = finished(&download_events);
    assert_eq!(
        saved.file_name().and_then(|name| name.to_str()),
        Some("report.pdf")
    );
    assert_eq!(std::fs::read(saved).expect("the download"), PDF_BODY);
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|recorded| recorded.path == "/attachment.pdf")
            .count(),
        2,
        "one request for the viewer's fetch and one for the download"
    );
}

#[test]
fn the_decision_follows_the_header_once_it_is_known() {
    let facts = |attachment| Facts {
        mime_type: "application/pdf",
        scheme: "http",
        user_started: true,
        foreground: true,
        posted: None,
        attachment,
        automated: false,
    };
    let before = pdf_disposition(
        PdfPreference::OpenInViewer,
        true,
        &facts(Attachment::Unknown),
    );
    let after = pdf_disposition(PdfPreference::OpenInViewer, true, &facts(Attachment::Yes));
    assert_ne!(before, after);
    assert_eq!(after, mercury_downloads::Disposition::Download);
}

#[test]
fn a_body_that_is_not_a_pdf_arrives_but_does_not_look_like_one() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-bad-body");
    let (manager, events) = manager();
    let id = manager.start(request(server.url("/not-a-pdf.pdf"), &dir.path));
    let events = events_until_done(&events, id);
    // The label says PDF, so the offer went to the viewer's fetch; the content decides.
    assert_eq!(started_mime(&events[0]).as_deref(), Some("application/pdf"));
    let path = finished(&events);
    assert!(!looks_like_pdf(&head_of(&path)));
}

#[test]
fn the_x_pdf_type_is_read_without_its_case_or_parameters() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-x-pdf");
    let (manager, events) = manager();
    let id = manager.start(request(server.url("/x-pdf"), &dir.path));
    let events = events_until_done(&events, id);
    // The engine lower-cases the type and drops its parameters.
    let mime = started_mime(&events[0]).expect("a content type");
    assert_eq!(mime, "application/x-pdf");
    assert!(is_pdf_mime(&mime));
}

#[test]
fn a_redirect_ends_at_the_pdf_and_the_final_address_is_reported() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-redirect");
    let (manager, events) = manager();
    let id = manager.start(request(server.url("/redirect-to-pdf"), &dir.path));
    let events = events_until_done(&events, id);
    match &events[0] {
        DownloadEvent::Started {
            final_url,
            attachment,
            ..
        } => {
            assert_eq!(final_url, &server.url("/doc.pdf"));
            assert!(!attachment);
        }
        other => panic!("expected Started, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(finished(&events)).expect("the file"),
        PDF_BODY
    );
    let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, vec!["/redirect-to-pdf", "/doc.pdf"]);
}

#[test]
fn cookies_referrer_and_user_agent_go_out_as_for_a_download() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-credentials");
    let (manager, events) = manager();
    let mut view = request(server.url("/doc.pdf"), &dir.path);
    view.cookie_header = Some("session=abc; theme=dark".to_owned());
    view.referrer = Some(url::Url::parse("http://127.0.0.1:9/page?secret=1").expect("a URL"));
    let id = manager.start(view);
    events_until_done(&events, id);

    let sent = server.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].header("cookie"), Some("session=abc; theme=dark"));
    assert_eq!(sent[0].header("user-agent"), Some("MercuryTest/1.0"));
    // Only the origin, never the query (strict-origin-when-cross-origin).
    assert_eq!(sent[0].header("referer"), Some("http://127.0.0.1:9/"));
}

#[test]
fn cookies_do_not_follow_a_redirect_to_another_host() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-cookie-redirect");
    let (manager, events) = manager();
    let mut view = request(server.url("/redirect-other-host"), &dir.path);
    view.cookie_header = Some("session=abc".to_owned());
    let id = manager.start(view);
    events_until_done(&events, id);
    let sent = server.requests();
    assert_eq!(sent[0].header("cookie"), Some("session=abc"));
    assert_eq!(
        sent[1].header("cookie"),
        None,
        "the second request went to another host name"
    );
}

#[test]
fn a_newer_offer_cancels_the_older_fetch_and_nothing_is_left_behind() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-cancelled");
    let (manager, events) = manager();

    let older = manager.start(request(server.url("/slow.pdf"), &dir.path));
    let started = wait_started(&events, older);
    assert_eq!(started_mime(&started).as_deref(), Some("application/pdf"));
    // The browser cancels the tab's older fetch when a newer offer for the tab arrives.
    manager.cancel(older);
    let older_events = events_until_done(&events, older);
    assert!(
        matches!(terminal(&older_events), DownloadEvent::Cancelled { .. }),
        "{older_events:?}"
    );
    assert_eq!(
        TempDir::list(&dir.path),
        Vec::<String>::new(),
        "the cancelled fetch's file is removed"
    );

    let newer = manager.start(request(server.url("/doc.pdf"), &dir.path));
    let newer_events = events_until_done(&events, newer);
    assert_eq!(
        std::fs::read(finished(&newer_events)).expect("the newer file"),
        PDF_BODY
    );
    assert_eq!(TempDir::list(&dir.path), vec!["doc.pdf".to_owned()]);
}

#[test]
fn a_missing_pdf_fails_without_a_started_event_or_a_file() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-missing");
    let (manager, events) = manager();
    let id = manager.start(request(server.url("/missing"), &dir.path));
    let events = events_until_done(&events, id);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], DownloadEvent::Failed { .. }));
    assert_eq!(TempDir::list(&dir.path), Vec::<String>::new());
}

fn failed_error(events: &[DownloadEvent]) -> &str {
    match terminal(events) {
        DownloadEvent::Failed {
            error, resumable, ..
        } => {
            assert!(!resumable, "a file over the limit is never kept to resume");
            error
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn a_pdf_that_never_ends_is_stopped_at_the_limit_and_leaves_no_file() {
    const LIMIT: u64 = 100_000;
    let server = TestServer::start();
    let dir = TempDir::new("pdf-endless");
    let (manager, events) = manager();
    let mut view = request(server.url("/endless.pdf"), &dir.path);
    view.max_bytes = Some(LIMIT);
    let id = manager.start(view);
    let events = events_until_done(&events, id);

    // The response gave no length, so it was accepted and a file was started ...
    assert!(
        matches!(events[0], DownloadEvent::Started { .. }),
        "{events:?}"
    );
    // ... and stopped when the body outgrew the limit, without writing past it.
    assert_eq!(
        failed_error(&events),
        "the file is larger than the 100000 bytes limit"
    );
    for event in &events {
        if let DownloadEvent::Progress { received_bytes, .. } = event {
            assert!(*received_bytes <= LIMIT, "{received_bytes} > {LIMIT}");
        }
    }
    assert_eq!(
        TempDir::list(&dir.path),
        Vec::<String>::new(),
        "neither the file nor its partial file is left"
    );
}

#[test]
fn a_pdf_that_says_it_is_too_big_is_refused_before_any_file_is_made() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-too-big");
    let (manager, events) = manager();
    // `/slow.pdf` promises 102,400 bytes.
    let mut view = request(server.url("/slow.pdf"), &dir.path);
    view.max_bytes = Some(50_000);
    let id = manager.start(view);
    let events = events_until_done(&events, id);

    assert_eq!(events.len(), 1, "no Started: {events:?}");
    assert_eq!(
        failed_error(&events),
        "the file is larger than the 50000 bytes limit"
    );
    assert_eq!(TempDir::list(&dir.path), Vec::<String>::new());
}

#[test]
fn the_limit_allows_exactly_its_size_and_no_limit_allows_anything() {
    let server = TestServer::start();
    let dir = TempDir::new("pdf-at-the-limit");
    let (manager, events) = manager();

    // Exactly the limit is allowed ...
    let mut view = request(server.url("/doc.pdf"), &dir.path);
    view.max_bytes = Some(PDF_BODY.len() as u64);
    let id = manager.start(view);
    let at_limit = events_until_done(&events, id);
    assert_eq!(
        std::fs::read(finished(&at_limit)).expect("the file"),
        PDF_BODY
    );

    // ... one byte less is not.
    let mut view = request(server.url("/doc.pdf"), &dir.path);
    view.max_bytes = Some(PDF_BODY.len() as u64 - 1);
    let id = manager.start(view);
    let under = events_until_done(&events, id);
    assert_eq!(under.len(), 1, "refused from its Content-Length: {under:?}");
    assert!(failed_error(&under).starts_with("the file is larger than the"));
    assert_eq!(TempDir::list(&dir.path), vec!["doc.pdf".to_owned()]);

    // No limit is no limit.
    let id = manager.start(request(server.url("/file.bin"), &dir.path));
    let unlimited = events_until_done(&events, id);
    assert_eq!(finished_len(&unlimited), 64 * 1024);
}

fn finished_len(events: &[DownloadEvent]) -> usize {
    std::fs::read(finished(events)).expect("the file").len()
}
