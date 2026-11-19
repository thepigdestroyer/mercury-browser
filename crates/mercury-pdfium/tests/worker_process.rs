// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The whole path: `PdfClient` -> pipe -> the helper process (`mercury-pdf-worker --pdf-worker`)
//! -> PDFium. The tests that need the real library skip without it (see `pdfium.rs`); the ones
//! about a missing library and about starting the helper by hand always run.

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use mercury_pdf::backend::PdfBackend;
use mercury_pdf::client::{ClientEvent, PdfClient, WorkerLaunch};
use mercury_pdf::error::PdfError;
use mercury_pdf::testing::{DocumentSpec, PageSpec, document_pdf};
use mercury_pdf::types::{FindOptions, LinkTarget, Rotation, TileRequest};
use mercury_pdfium::PdfiumBackend;
use support::{Scratch, encrypted_pdf, fixture, library};

const WORKER: &str = env!("CARGO_BIN_EXE_mercury-pdf-worker");
const WAIT: Duration = Duration::from_secs(60);

fn launch(library: Option<PathBuf>) -> WorkerLaunch {
    WorkerLaunch {
        program: PathBuf::from(WORKER),
        args: vec![OsString::from("--pdf-worker")],
        library,
    }
}

fn start(library: Option<PathBuf>) -> (PdfClient, Receiver<ClientEvent>) {
    let (sender, receiver) = mpsc::channel();
    let sender = Mutex::new(sender);
    let client = PdfClient::start(
        launch(library),
        Box::new(move |event| {
            let _ = sender.lock().unwrap().send(event);
        }),
    )
    .expect("the helper starts");
    (client, receiver)
}

fn next(receiver: &Receiver<ClientEvent>) -> ClientEvent {
    receiver.recv_timeout(WAIT).expect("an event in time")
}

fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn resident_kilobytes(pid: u32) -> u64 {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

#[test]
fn started_by_hand_without_the_flag_it_explains_itself_and_exits() {
    let output = Command::new(WORKER)
        .stdin(Stdio::null())
        .output()
        .expect("the program starts");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "nothing on standard output");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--pdf-worker"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_helper_that_cannot_load_the_library_says_why_and_the_loss_carries_it() {
    let (client, events) = start(Some(PathBuf::from("/nonexistent/libpdfium.dylib")));
    let pid = client.process_id().unwrap();
    client.open(Path::new("/fake/file.pdf"), None);
    let ClientEvent::WorkerLost(error) = next(&events) else {
        panic!("expected the loss");
    };
    assert!(matches!(error, PdfError::LibraryMissing(_)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "Mercury's PDF viewer is not available in this copy."
    );
    assert!(matches!(
        events.recv_timeout(Duration::from_millis(300)),
        Err(RecvTimeoutError::Timeout)
    ));
    drop(client);
    assert!(!process_exists(pid));
}

#[test]
fn a_document_through_the_real_helper() {
    let Some(library) = library() else { return };
    let (client, events) = start(Some(library.clone()));
    let pid = client.process_id().unwrap();
    let path = fixture("three-pages.pdf");
    client.open(&path, None);
    let ClientEvent::Opened(Ok(info)) = next(&events) else {
        panic!("expected Opened");
    };
    assert_eq!(info.page_count(), 3);
    assert_eq!(info.title.as_deref(), Some("Mercury three-page test"));

    // A tile through the pipe is the tile the backend renders directly.
    let request = TileRequest {
        page: 1,
        scale: 1.5,
        rotation: Rotation::Deg0,
        x: 100,
        y: 50,
        width: 300,
        height: 200,
    };
    let id = client.request_tile(request, 1);
    let ClientEvent::Tile {
        id: got, result, ..
    } = next(&events)
    else {
        panic!("expected a tile");
    };
    assert_eq!(got, id);
    let mut direct = PdfiumBackend::load(&library).unwrap();
    direct.open(&path, None).unwrap();
    assert_eq!(result.unwrap(), direct.render(&request).unwrap());

    client.request_page_text(2);
    let ClientEvent::PageText { page, result } = next(&events) else {
        panic!("expected text");
    };
    assert_eq!(page, 2);
    assert_eq!(result.unwrap(), direct.page_text(2).unwrap());

    client.request_links(0);
    let ClientEvent::Links { result, .. } = next(&events) else {
        panic!("expected links");
    };
    let links = result.unwrap();
    assert_eq!(links.len(), 2);
    assert!(matches!(&links[0].target, LinkTarget::Uri(uri) if uri.contains("example.com")));

    client.request_outline();
    let ClientEvent::Outline(result) = next(&events) else {
        panic!("expected the outline");
    };
    assert_eq!(result.unwrap().len(), 2);

    // A find starts at page 2 and goes round.
    let find = client.find("fox", FindOptions::default(), 2);
    let mut pages = Vec::new();
    loop {
        let ClientEvent::Found {
            find: got,
            page,
            hits,
            finished,
        } = next(&events)
        else {
            panic!("expected hits");
        };
        assert_eq!(got, find);
        assert_eq!(hits.len(), 2);
        pages.push(page);
        if finished {
            break;
        }
    }
    assert_eq!(pages, [2, 0, 1]);

    drop(client);
    assert!(!process_exists(pid), "no helper is left after the drop");
}

#[test]
fn a_password_through_the_real_helper() {
    let Some(library) = library() else { return };
    let scratch = Scratch::new("process-password");
    let path = scratch.write("locked.pdf", &encrypted_pdf("open sesame"));
    let (client, events) = start(Some(library));
    client.open(&path, None);
    assert!(matches!(
        next(&events),
        ClientEvent::Opened(Err(PdfError::PasswordRequired))
    ));
    client.open(&path, Some("wrong"));
    assert!(matches!(
        next(&events),
        ClientEvent::Opened(Err(PdfError::PasswordIncorrect))
    ));
    client.open(&path, Some("open sesame"));
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
}

#[test]
fn a_helper_started_with_a_corrupt_file_survives_and_reports() {
    let Some(library) = library() else { return };
    let scratch = Scratch::new("process-corrupt");
    let path = scratch.write("junk.pdf", b"not a pdf\n");
    let (client, events) = start(Some(library));
    client.open(&path, None);
    assert!(matches!(
        next(&events),
        ClientEvent::Opened(Err(PdfError::Corrupt(_)))
    ));
    assert!(!client.has_ended());
    client.open(&fixture("three-pages.pdf"), None);
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
}

#[test]
fn the_helper_ignores_interrupt_hangup_and_terminate() {
    let Some(library) = library() else { return };
    let (client, events) = start(Some(library));
    let pid = client.process_id().unwrap();
    client.open(&fixture("three-pages.pdf"), None);
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
    for signal in ["INT", "HUP", "TERM"] {
        let status = Command::new("kill")
            .args(["-s", signal, &pid.to_string()])
            .status()
            .expect("kill runs");
        assert!(status.success());
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        process_exists(pid),
        "the helper is still there after the signals"
    );
    // And it still answers.
    client.request_page_text(0);
    assert!(matches!(
        next(&events),
        ClientEvent::PageText { result: Ok(_), .. }
    ));
    assert!(!client.has_ended());
    // The browser ends it with a kill, which no helper can ignore.
    drop(client);
    assert!(!process_exists(pid));
}

/// Not a check: prints the numbers the lane report quotes (time to the first tile, a tile for
/// each of 200 pages, a find across them, and the helper's memory afterwards).
#[test]
fn numbers_for_a_200_page_document() {
    let Some(library) = library() else { return };
    let scratch = Scratch::new("two-hundred");
    let lines: Vec<String> = (1..=40)
        .map(|line| {
            format!("Line {line}: the quick brown fox jumps over the lazy dog, again and again")
        })
        .collect();
    let pages: Vec<PageSpec> = (0..200)
        .map(|index| {
            let mut page = PageSpec::letter(lines.clone());
            page.lines[0] = format!("Page {} of 200", index + 1);
            page
        })
        .collect();
    let bytes = document_pdf(&DocumentSpec {
        title: Some("Two hundred pages".to_owned()),
        pages,
        outline: Vec::new(),
    });
    let path = scratch.write("two-hundred.pdf", &bytes);
    eprintln!("the 200-page file is {} bytes", bytes.len());

    let started = Instant::now();
    let (client, events) = start(Some(library));
    let pid = client.process_id().unwrap();
    client.open(&path, None);
    let ClientEvent::Opened(Ok(info)) = next(&events) else {
        panic!("expected Opened");
    };
    let opened = started.elapsed();
    assert_eq!(info.page_count(), 200);
    let tile = |page: u32, scale: f32| TileRequest {
        page,
        scale,
        rotation: Rotation::Deg0,
        x: 0,
        y: 0,
        width: 512,
        height: 512,
    };
    let first_start = Instant::now();
    client.request_tile(tile(0, 1.0), 1);
    assert!(matches!(
        next(&events),
        ClientEvent::Tile { result: Ok(_), .. }
    ));
    let first = first_start.elapsed();
    eprintln!(
        "start + open of 200 pages: {opened:?}; first 512x512 tile after that: {first:?}; \
         both together from a cold process: {:?}",
        started.elapsed()
    );

    let every_page = Instant::now();
    for page in 0..200 {
        client.request_tile(tile(page, 1.0), 1);
    }
    for _ in 0..200 {
        assert!(matches!(
            next(&events),
            ClientEvent::Tile { result: Ok(_), .. }
        ));
    }
    eprintln!(
        "a 512x512 tile of each of 200 pages at 1x: {:?} in all, {:?} each",
        every_page.elapsed(),
        every_page.elapsed() / 200
    );
    let every_page_2x = Instant::now();
    for page in 0..200 {
        client.request_tile(tile(page, 2.0), 1);
    }
    for _ in 0..200 {
        assert!(matches!(
            next(&events),
            ClientEvent::Tile { result: Ok(_), .. }
        ));
    }
    eprintln!(
        "a 512x512 tile of each of 200 pages at 2x: {:?} in all, {:?} each",
        every_page_2x.elapsed(),
        every_page_2x.elapsed() / 200
    );

    let find_started = Instant::now();
    let find = client.find("lazy dog", FindOptions::default(), 0);
    let mut total_hits = 0;
    loop {
        let ClientEvent::Found {
            find: got,
            hits,
            finished,
            ..
        } = next(&events)
        else {
            panic!("expected hits");
        };
        assert_eq!(got, find);
        total_hits += hits.len();
        if finished {
            break;
        }
    }
    eprintln!(
        "a find for 'lazy dog' across 200 pages: {} hits in {:?}",
        total_hits,
        find_started.elapsed()
    );
    // 39 lines of each page say it; the first line of each page was replaced.
    assert_eq!(total_hits, 200 * 39);

    let text_started = Instant::now();
    client.request_page_text(100);
    assert!(matches!(
        next(&events),
        ClientEvent::PageText { result: Ok(_), .. }
    ));
    eprintln!(
        "the text of one page of 40 lines: {:?}",
        text_started.elapsed()
    );

    let kilobytes = resident_kilobytes(pid);
    eprintln!(
        "helper memory after the 200-page file: {} MiB resident",
        kilobytes / 1024
    );
    assert!(kilobytes / 1024 < 600, "{kilobytes} KiB");
    drop(client);
    assert!(!process_exists(pid));
}
