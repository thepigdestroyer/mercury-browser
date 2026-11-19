// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The helper's request loop (`mercury_pdf::serve`) through in-memory pipes with the fake
//! backend: ordering, cancellation, find, the password flow, validation, cleaning, the panic
//! status and malformed input.

use std::io::Cursor;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mercury_pdf::backend::PdfBackend;
use mercury_pdf::error::PdfError;
use mercury_pdf::fake::{FakeBackend, FakeConfig, expected_tile, fake_page_text};
use mercury_pdf::protocol::{
    MAX_HEADER_BYTES, PROTOCOL_VERSION, Reply, Request, decode_list, decode_page_sizes,
    decode_page_text, read_reply, write_request,
};
use mercury_pdf::serve::{EXIT_PANIC, ServeExit, serve};
use mercury_pdf::testing::{ServeHarness, pipe};
use mercury_pdf::types::{
    DocumentInfo, FindOptions, Link, LinkTarget, OutlineItem, PageSize, PageText, Rect, Rotation,
    SearchHit, Tile, TileRequest,
};

/// A short pause for the reader thread to take what was just sent (the pipe is in memory; this
/// is only about thread scheduling).
const SETTLE: Duration = Duration::from_millis(150);

fn tile(page: u32, x: u32) -> TileRequest {
    TileRequest {
        page,
        scale: 1.0,
        rotation: Rotation::Deg0,
        x,
        y: 0,
        width: 8,
        height: 8,
    }
}

fn open(id: u64) -> Request {
    Request::Open {
        id,
        path: "/fake/file.pdf".to_owned(),
        password: None,
    }
}

fn tile_request(id: u64, priority: u8, request: TileRequest) -> Request {
    Request::Tile {
        id,
        priority,
        request,
    }
}

/// A backend that logs what it is asked and can stop in the middle of one call until the test
/// lets it go, so the test can queue requests behind a busy backend.
struct Probe {
    inner: FakeBackend,
    log: Arc<Mutex<Vec<String>>>,
    calls: usize,
    gate: Option<Gate>,
}

struct Gate {
    /// The call (counted from 1, renders, text, find, links and outline) that waits.
    call: usize,
    reached: Sender<()>,
    release: Receiver<()>,
}

struct GateControl {
    reached: Receiver<()>,
    release: Sender<()>,
}

impl Probe {
    fn new(config: FakeConfig) -> (Self, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner: FakeBackend::new(config),
                log: Arc::clone(&log),
                calls: 0,
                gate: None,
            },
            log,
        )
    }

    fn gated(config: FakeConfig, call: usize) -> (Self, Arc<Mutex<Vec<String>>>, GateControl) {
        let (mut probe, log) = Self::new(config);
        let (reached_sender, reached) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel();
        probe.gate = Some(Gate {
            call,
            reached: reached_sender,
            release: release_receiver,
        });
        (probe, log, GateControl { reached, release })
    }

    fn enter(&mut self, what: String) {
        self.calls += 1;
        self.log.lock().unwrap().push(what);
        if let Some(gate) = &self.gate
            && gate.call == self.calls
        {
            gate.reached.send(()).unwrap();
            gate.release.recv().unwrap();
        }
    }
}

impl PdfBackend for Probe {
    fn open(&mut self, path: &Path, password: Option<&str>) -> Result<DocumentInfo, PdfError> {
        self.inner.open(path, password)
    }
    fn render(&mut self, request: &TileRequest) -> Result<Tile, PdfError> {
        self.enter(format!("render p{} x{}", request.page, request.x));
        self.inner.render(request)
    }
    fn page_text(&mut self, page: u32) -> Result<PageText, PdfError> {
        self.enter(format!("text p{page}"));
        self.inner.page_text(page)
    }
    fn find_in_page(
        &mut self,
        page: u32,
        query: &str,
        options: FindOptions,
    ) -> Result<Vec<SearchHit>, PdfError> {
        self.enter(format!("find p{page}"));
        self.inner.find_in_page(page, query, options)
    }
    fn links(&mut self, page: u32) -> Result<Vec<Link>, PdfError> {
        self.enter(format!("links p{page}"));
        self.inner.links(page)
    }
    fn outline(&mut self) -> Result<Vec<OutlineItem>, PdfError> {
        self.enter("outline".to_owned());
        self.inner.outline()
    }
    fn close(&mut self) {
        self.log.lock().unwrap().push("close".to_owned());
        self.inner.close();
    }
}

fn ready(harness: &mut ServeHarness) {
    let (reply, blob) = harness.recv();
    assert_eq!(
        reply,
        Reply::Ready {
            protocol_version: PROTOCOL_VERSION,
            engine: "mercury-pdf".to_owned()
        }
    );
    assert!(blob.is_empty());
}

fn opened_harness(config: FakeConfig) -> ServeHarness {
    let mut harness = ServeHarness::start(FakeBackend::new(config));
    ready(&mut harness);
    harness.send(&open(1));
    let (reply, _) = harness.recv();
    assert!(
        matches!(reply, Reply::Opened { error: None, .. }),
        "{reply:?}"
    );
    harness
}

fn tile_id(reply: &Reply) -> u64 {
    match reply {
        Reply::Tile { id, .. } => *id,
        other => panic!("expected a tile reply, got {other:?}"),
    }
}

fn logged(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().unwrap().clone()
}

#[test]
fn the_first_frame_is_ready_and_open_answers_with_the_page_sizes() {
    let mut harness = ServeHarness::start(FakeBackend::new(FakeConfig {
        title: Some("A title".to_owned()),
        ..FakeConfig::with_pages(4)
    }));
    ready(&mut harness);
    harness.send(&open(7));
    let (reply, blob) = harness.recv();
    let Reply::Opened {
        id,
        title,
        page_count,
        error,
    } = reply
    else {
        panic!("expected Opened");
    };
    assert_eq!((id, page_count, error), (7, 4, None));
    assert_eq!(title.as_deref(), Some("A title"));
    let pages = decode_page_sizes(&blob, page_count).unwrap();
    assert_eq!(pages, vec![PageSize::new(612.0, 792.0); 4]);
    assert_eq!(harness.finish(), ServeExit::Eof);
}

#[test]
fn a_tile_comes_back_with_its_pixels_in_the_blob() {
    let mut harness = opened_harness(FakeConfig::default());
    let request = tile(2, 24);
    harness.send(&tile_request(5, 0, request));
    let (reply, blob) = harness.recv();
    assert_eq!(
        reply,
        Reply::Tile {
            id: 5,
            width: 8,
            height: 8,
            error: None
        }
    );
    assert_eq!(blob, expected_tile(&request).rgba);
}

#[test]
fn text_links_outline_come_back_in_their_blobs() {
    let mut harness = opened_harness(FakeConfig::default());
    harness.send(&Request::PageText { id: 2, page: 1 });
    let (reply, blob) = harness.recv();
    let Reply::PageText {
        id, page, chars, ..
    } = reply
    else {
        panic!("expected PageText");
    };
    assert_eq!((id, page), (2, 1));
    assert_eq!(decode_page_text(&blob, chars).unwrap(), fake_page_text(1));

    harness.send(&Request::Links { id: 3, page: 0 });
    let (reply, blob) = harness.recv();
    assert!(matches!(
        reply,
        Reply::Links {
            id: 3,
            page: 0,
            error: None
        }
    ));
    let links: Vec<Link> = decode_list(&blob).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].target,
        LinkTarget::Page {
            page: 1,
            y: Some(0.0)
        }
    );

    harness.send(&Request::Outline { id: 4 });
    let (reply, blob) = harness.recv();
    assert!(matches!(reply, Reply::Outline { id: 4, error: None }));
    let outline: Vec<OutlineItem> = decode_list(&blob).unwrap();
    assert_eq!(outline[0].title, "Chapter 1");
}

#[test]
fn nothing_works_before_a_document_is_open() {
    let mut harness = ServeHarness::start(FakeBackend::default());
    ready(&mut harness);
    harness.send(&tile_request(1, 0, tile(0, 0)));
    harness.send(&Request::PageText { id: 2, page: 0 });
    harness.send(&Request::Links { id: 3, page: 0 });
    harness.send(&Request::Outline { id: 4 });
    harness.send(&Request::Find {
        id: 5,
        query: "fox".to_owned(),
        options: FindOptions::default(),
        from_page: 0,
    });
    let mut errors = Vec::new();
    for _ in 0..5 {
        let (reply, _) = harness.recv();
        let error = match reply {
            Reply::Tile { error, .. }
            | Reply::PageText { error, .. }
            | Reply::Links { error, .. }
            | Reply::Outline { error, .. }
            | Reply::Found { error, .. } => error,
            other => panic!("unexpected {other:?}"),
        };
        errors.push(error);
    }
    assert!(
        errors
            .iter()
            .all(|error| *error == Some(PdfError::NoDocument))
    );
    assert_eq!(harness.finish(), ServeExit::Eof);
}

#[test]
fn a_second_open_is_refused_and_the_document_keeps_working() {
    let mut harness = opened_harness(FakeConfig::default());
    harness.send(&open(2));
    let (reply, _) = harness.recv();
    assert!(matches!(
        reply,
        Reply::Opened {
            id: 2,
            error: Some(PdfError::InvalidRequest(_)),
            ..
        }
    ));
    harness.send(&tile_request(3, 0, tile(0, 0)));
    assert!(matches!(harness.recv().0, Reply::Tile { error: None, .. }));
}

#[test]
fn the_password_flow_asks_rejects_and_accepts() {
    let mut harness = ServeHarness::start(FakeBackend::new(FakeConfig {
        password: Some("sesame".to_owned()),
        ..FakeConfig::default()
    }));
    ready(&mut harness);
    for (id, password, wanted) in [
        (1, None, Some(PdfError::PasswordRequired)),
        (2, Some("nope"), Some(PdfError::PasswordIncorrect)),
        (3, Some("sesame"), None),
    ] {
        harness.send(&Request::Open {
            id,
            path: "/fake/locked.pdf".to_owned(),
            password: password.map(str::to_owned),
        });
        let (reply, _) = harness.recv();
        let Reply::Opened { id: got, error, .. } = reply else {
            panic!("expected Opened");
        };
        assert_eq!((got, error), (id, wanted));
    }
    harness.send(&tile_request(4, 0, tile(0, 0)));
    assert!(matches!(harness.recv().0, Reply::Tile { error: None, .. }));
}

#[test]
fn a_damaged_file_is_an_error_and_not_a_crash() {
    let mut harness = ServeHarness::start(FakeBackend::new(FakeConfig {
        corrupt: true,
        ..FakeConfig::default()
    }));
    ready(&mut harness);
    harness.send(&open(1));
    assert!(matches!(
        harness.recv().0,
        Reply::Opened {
            error: Some(PdfError::Corrupt(_)),
            ..
        }
    ));
    assert_eq!(harness.finish(), ServeExit::Eof);
}

#[test]
fn a_request_that_breaks_a_rule_is_answered_with_an_error_and_the_loop_goes_on() {
    let mut harness = opened_harness(FakeConfig::default());
    let broken = [
        TileRequest {
            page: 9,
            ..tile(0, 0)
        },
        TileRequest {
            scale: 99.0,
            ..tile(0, 0)
        },
        TileRequest {
            x: 10_000,
            ..tile(0, 0)
        },
        TileRequest {
            width: 0,
            ..tile(0, 0)
        },
    ];
    for (index, request) in broken.into_iter().enumerate() {
        harness.send(&tile_request(index as u64 + 10, 0, request));
        let (reply, blob) = harness.recv();
        assert!(
            matches!(
                reply,
                Reply::Tile {
                    error: Some(PdfError::InvalidRequest(_)),
                    ..
                }
            ),
            "{reply:?}"
        );
        assert!(blob.is_empty());
    }
    harness.send(&Request::PageText { id: 20, page: 99 });
    assert!(matches!(
        harness.recv().0,
        Reply::PageText {
            error: Some(PdfError::InvalidRequest(_)),
            ..
        }
    ));
    harness.send(&Request::Links { id: 21, page: 3 });
    assert!(matches!(
        harness.recv().0,
        Reply::Links {
            error: Some(PdfError::InvalidRequest(_)),
            ..
        }
    ));
    harness.send(&tile_request(30, 0, tile(1, 0)));
    assert!(matches!(harness.recv().0, Reply::Tile { error: None, .. }));
}

#[test]
fn tiles_run_by_priority_and_then_in_arrival_order() {
    let (probe, log, gate) = Probe::gated(FakeConfig::default(), 1);
    let mut harness = ServeHarness::start(probe);
    ready(&mut harness);
    harness.send(&open(1));
    harness.recv();
    // The first tile occupies the backend; the rest queue up behind it.
    harness.send(&tile_request(10, 0, tile(0, 0)));
    gate.reached.recv_timeout(Duration::from_secs(10)).unwrap();
    for (id, priority, x) in [(11, 10, 8), (12, 50, 16), (13, 50, 24), (14, 10, 32)] {
        harness.send(&tile_request(id, priority, tile(0, x)));
    }
    std::thread::sleep(SETTLE);
    gate.release.send(()).unwrap();
    let mut order = Vec::new();
    for _ in 0..5 {
        order.push(tile_id(&harness.recv().0));
    }
    assert_eq!(order, [10, 12, 13, 11, 14]);
    assert_eq!(
        logged(&log),
        [
            "render p0 x0",
            "render p0 x16",
            "render p0 x24",
            "render p0 x8",
            "render p0 x32"
        ]
    );
}

#[test]
fn a_cancelled_tile_never_runs_and_is_answered_as_cancelled() {
    let (probe, log, gate) = Probe::gated(FakeConfig::default(), 1);
    let mut harness = ServeHarness::start(probe);
    ready(&mut harness);
    harness.send(&open(1));
    harness.recv();
    harness.send(&tile_request(10, 0, tile(0, 0)));
    gate.reached.recv_timeout(Duration::from_secs(10)).unwrap();
    harness.send(&tile_request(11, 5, tile(0, 8)));
    harness.send(&tile_request(12, 5, tile(0, 16)));
    harness.send(&Request::Cancel { id: 11 });
    // A cancel for something that does not exist, or already ended, is nothing.
    harness.send(&Request::Cancel { id: 999 });
    std::thread::sleep(SETTLE);
    gate.release.send(()).unwrap();
    let first = harness.recv().0;
    let second = harness.recv().0;
    let third = harness.recv().0;
    assert!(matches!(
        first,
        Reply::Tile {
            id: 10,
            error: None,
            ..
        }
    ));
    assert!(
        matches!(
            second,
            Reply::Tile {
                id: 11,
                error: Some(PdfError::Cancelled),
                ..
            }
        ),
        "{second:?}"
    );
    assert!(matches!(
        third,
        Reply::Tile {
            id: 12,
            error: None,
            ..
        }
    ));
    assert_eq!(logged(&log), ["render p0 x0", "render p0 x16"]);
}

#[test]
fn a_find_goes_page_by_page_and_wraps_round() {
    let mut harness = opened_harness(FakeConfig::with_pages(4));
    harness.send(&Request::Find {
        id: 9,
        query: "Fox".to_owned(),
        options: FindOptions::default(),
        from_page: 2,
    });
    let mut pages = Vec::new();
    loop {
        let (reply, blob) = harness.recv();
        let Reply::Found {
            find,
            page,
            finished,
            error,
        } = reply
        else {
            panic!("expected Found, got {reply:?}");
        };
        assert_eq!((find, error), (9, None));
        let hits: Vec<SearchHit> = decode_list(&blob).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page, page);
        pages.push(page);
        assert_eq!(finished, pages.len() == 4);
        if finished {
            break;
        }
    }
    assert_eq!(pages, [2, 3, 0, 1]);
}

#[test]
fn find_options_reach_the_backend_and_an_empty_query_finishes_at_once() {
    let mut harness = opened_harness(FakeConfig::with_pages(2));
    harness.send(&Request::Find {
        id: 1,
        query: "Fox".to_owned(),
        options: FindOptions {
            match_case: true,
            whole_word: false,
        },
        from_page: 0,
    });
    let mut count = 0;
    loop {
        let (reply, blob) = harness.recv();
        let Reply::Found { finished, .. } = reply else {
            panic!("expected Found");
        };
        assert!(decode_list::<SearchHit>(&blob).unwrap().is_empty());
        count += 1;
        if finished {
            break;
        }
    }
    assert_eq!(count, 2);
    harness.send(&Request::Find {
        id: 2,
        query: String::new(),
        options: FindOptions::default(),
        from_page: 0,
    });
    let (reply, blob) = harness.recv();
    assert!(matches!(
        reply,
        Reply::Found {
            find: 2,
            finished: true,
            error: None,
            ..
        }
    ));
    assert!(decode_list::<SearchHit>(&blob).unwrap().is_empty());
}

#[test]
fn a_find_that_breaks_a_rule_is_one_final_error() {
    let mut harness = opened_harness(FakeConfig::with_pages(2));
    harness.send(&Request::Find {
        id: 1,
        query: "x".repeat(300),
        options: FindOptions::default(),
        from_page: 0,
    });
    assert!(matches!(
        harness.recv().0,
        Reply::Found {
            find: 1,
            finished: true,
            error: Some(PdfError::InvalidRequest(_)),
            ..
        }
    ));
    harness.send(&Request::Find {
        id: 2,
        query: "fox".to_owned(),
        options: FindOptions::default(),
        from_page: 7,
    });
    assert!(matches!(
        harness.recv().0,
        Reply::Found {
            find: 2,
            finished: true,
            error: Some(PdfError::InvalidRequest(_)),
            ..
        }
    ));
    // And the loop still works.
    harness.send(&Request::Outline { id: 3 });
    assert!(matches!(
        harness.recv().0,
        Reply::Outline { error: None, .. }
    ));
}

#[test]
fn a_find_gives_way_to_tiles_between_its_pages() {
    // Calls: 1 = find page 0, 2 = find page 1 (the gate).
    let (probe, log, gate) = Probe::gated(FakeConfig::with_pages(4), 2);
    let mut harness = ServeHarness::start(probe);
    ready(&mut harness);
    harness.send(&open(1));
    harness.recv();
    harness.send(&Request::Find {
        id: 9,
        query: "fox".to_owned(),
        options: FindOptions::default(),
        from_page: 0,
    });
    gate.reached.recv_timeout(Duration::from_secs(10)).unwrap();
    // The backend is inside page 1 of the find; two tiles arrive now.
    harness.send(&tile_request(20, 0, tile(0, 0)));
    harness.send(&tile_request(21, 0, tile(0, 8)));
    std::thread::sleep(SETTLE);
    gate.release.send(()).unwrap();
    let mut seen = Vec::new();
    while seen.len() < 6 {
        seen.push(harness.recv().0);
    }
    assert_eq!(
        logged(&log),
        [
            "find p0",
            "find p1",
            "render p0 x0",
            "render p0 x8",
            "find p2",
            "find p3"
        ]
    );
    // The find was not answered out of order or lost: four pages, the last one finished.
    let found: Vec<(u32, bool)> = seen
        .iter()
        .filter_map(|reply| match reply {
            Reply::Found { page, finished, .. } => Some((*page, *finished)),
            _ => None,
        })
        .collect();
    assert_eq!(found, [(0, false), (1, false), (2, false), (3, true)]);
}

#[test]
fn a_cancelled_find_stops_after_at_most_the_page_in_progress() {
    let mut harness = opened_harness(FakeConfig {
        find_delay: Duration::from_millis(40),
        ..FakeConfig::with_pages(50)
    });
    harness.send(&Request::Find {
        id: 9,
        query: "fox".to_owned(),
        options: FindOptions::default(),
        from_page: 0,
    });
    assert!(matches!(harness.recv().0, Reply::Found { page: 0, .. }));
    harness.send(&Request::CancelFind { id: 9 });
    std::thread::sleep(Duration::from_millis(200));
    // A sentinel after the cancel: everything before its reply belongs to the find.
    harness.send(&Request::Outline { id: 1 });
    let (skipped, _) = harness.recv_until(|reply| matches!(reply, Reply::Outline { .. }));
    assert!(
        skipped.len() <= 2,
        "{} answers after the cancel",
        skipped.len()
    );
    // No more answers arrive afterwards.
    harness.send(&Request::Outline { id: 2 });
    assert!(matches!(harness.recv().0, Reply::Outline { id: 2, .. }));
}

#[test]
fn open_and_close_are_barriers() {
    let (probe, log, gate) = Probe::gated(FakeConfig::default(), 1);
    let mut harness = ServeHarness::start(probe);
    ready(&mut harness);
    harness.send(&open(1));
    harness.recv();
    harness.send(&tile_request(10, 0, tile(0, 0)));
    gate.reached.recv_timeout(Duration::from_secs(10)).unwrap();
    // Close is sent after the first tile, a second tile (with the best priority) after Close.
    harness.send(&Request::Close { id: 11 });
    harness.send(&tile_request(12, 255, tile(0, 8)));
    harness.send(&open(13));
    harness.send(&tile_request(14, 0, tile(0, 16)));
    std::thread::sleep(SETTLE);
    gate.release.send(()).unwrap();
    let replies: Vec<Reply> = (0..5).map(|_| harness.recv().0).collect();
    assert!(matches!(
        replies[0],
        Reply::Tile {
            id: 10,
            error: None,
            ..
        }
    ));
    assert_eq!(replies[1], Reply::Closed { id: 11 });
    assert!(
        matches!(
            replies[2],
            Reply::Tile {
                id: 12,
                error: Some(PdfError::NoDocument),
                ..
            }
        ),
        "{:?}",
        replies[2]
    );
    assert!(matches!(
        replies[3],
        Reply::Opened {
            id: 13,
            error: None,
            ..
        }
    ));
    assert!(matches!(
        replies[4],
        Reply::Tile {
            id: 14,
            error: None,
            ..
        }
    ));
    assert_eq!(
        logged(&log),
        ["render p0 x0", "close", "render p0 x16"],
        "the tile asked after Close did not reach the backend"
    );
}

#[test]
fn a_barrier_ends_a_find_that_belonged_to_the_old_document() {
    let mut harness = opened_harness(FakeConfig {
        find_delay: Duration::from_millis(30),
        ..FakeConfig::with_pages(30)
    });
    harness.send(&Request::Find {
        id: 9,
        query: "fox".to_owned(),
        options: FindOptions::default(),
        from_page: 0,
    });
    assert!(matches!(harness.recv().0, Reply::Found { .. }));
    harness.send(&Request::Close { id: 2 });
    let (skipped, closed) = harness.recv_until(|reply| matches!(reply, Reply::Closed { .. }));
    assert!(skipped.len() <= 2);
    assert_eq!(closed.0, Reply::Closed { id: 2 });
    harness.send(&Request::Outline { id: 3 });
    // The next answer is the outline's (an error: no document), not another page of the find.
    assert!(matches!(
        harness.recv().0,
        Reply::Outline {
            id: 3,
            error: Some(PdfError::NoDocument)
        }
    ));
}

#[test]
fn a_backend_panic_is_caught_answered_and_ends_the_loop_with_status_70() {
    let mut harness = opened_harness(FakeConfig {
        panic_after: Some(1),
        ..FakeConfig::default()
    });
    harness.send(&tile_request(1, 0, tile(0, 0)));
    assert!(matches!(harness.recv().0, Reply::Tile { error: None, .. }));
    harness.send(&tile_request(2, 0, tile(0, 8)));
    let (reply, _) = harness.recv();
    assert!(
        matches!(
            reply,
            Reply::Tile {
                id: 2,
                error: Some(PdfError::WorkerLost(_)),
                ..
            }
        ),
        "{reply:?}"
    );
    // The stream ends: nothing else is written after the answer.
    assert!(harness.recv_opt().is_none());
    let exit = harness.finish();
    assert!(matches!(exit, ServeExit::Panicked(_)), "{exit:?}");
    assert_eq!(exit.status(), EXIT_PANIC);
    assert_eq!(EXIT_PANIC, 70);
}

#[test]
fn malformed_input_gets_one_error_reply_and_an_orderly_end() {
    let wires: Vec<(&str, Vec<u8>)> = vec![
        ("header too long", {
            let mut bytes = (MAX_HEADER_BYTES as u32 + 1).to_le_bytes().to_vec();
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes
        }),
        ("huge header", {
            let mut bytes = u32::MAX.to_le_bytes().to_vec();
            bytes.extend_from_slice(&u32::MAX.to_le_bytes());
            bytes
        }),
        ("blob too long", {
            let mut bytes = 2_u32.to_le_bytes().to_vec();
            bytes.extend_from_slice(&u32::MAX.to_le_bytes());
            bytes.extend_from_slice(b"{}");
            bytes
        }),
        ("empty header", vec![0; 8]),
        ("not json", {
            let mut bytes = 8_u32.to_le_bytes().to_vec();
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes.extend_from_slice(b"not json");
            bytes
        }),
        ("unknown message", {
            let body = br#"{"type":"explode"}"#;
            let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes.extend_from_slice(body);
            bytes
        }),
        ("a blob on a request", {
            let body = br#"{"type":"outline","id":1}"#;
            let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
            bytes.extend_from_slice(&3_u32.to_le_bytes());
            bytes.extend_from_slice(body);
            bytes.extend_from_slice(b"abc");
            bytes
        }),
        ("truncated", {
            let mut bytes = 20_u32.to_le_bytes().to_vec();
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes.extend_from_slice(b"{\"ty");
            bytes
        }),
    ];
    for (name, wire) in wires {
        let mut harness = ServeHarness::start(FakeBackend::default());
        ready(&mut harness);
        harness.send_raw(&wire);
        if name == "truncated" {
            // A truncated frame is only noticed when the stream ends.
            harness.close_input();
        }
        let (reply, blob) = harness.recv();
        assert!(
            matches!(
                reply,
                Reply::Error {
                    id: None,
                    error: PdfError::InvalidRequest(_)
                }
            ),
            "{name}: {reply:?}"
        );
        assert!(blob.is_empty());
        assert!(harness.recv_opt().is_none(), "{name}: the stream ends");
        let exit = harness.finish();
        assert!(matches!(exit, ServeExit::Protocol(_)), "{name}: {exit:?}");
        assert_eq!(exit.status(), 0, "{name}: an orderly end");
    }
}

#[test]
fn the_end_of_the_input_answers_everything_asked_first_and_writes_only_frames() {
    let mut input = Vec::new();
    write_request(&mut input, &open(1)).unwrap();
    write_request(&mut input, &tile_request(2, 0, tile(1, 0))).unwrap();
    write_request(&mut input, &Request::PageText { id: 3, page: 0 }).unwrap();
    write_request(
        &mut input,
        &Request::Find {
            id: 4,
            query: "dog".to_owned(),
            options: FindOptions::default(),
            from_page: 0,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    let exit = serve(FakeBackend::default(), Cursor::new(input), &mut output);
    assert_eq!(exit, ServeExit::Eof);
    // Every byte of the output belongs to a frame.
    let mut cursor = Cursor::new(output);
    let mut replies = Vec::new();
    while let Some((reply, _)) = read_reply(&mut cursor).unwrap() {
        replies.push(reply);
    }
    assert!(matches!(replies[0], Reply::Ready { .. }));
    assert!(matches!(
        replies[1],
        Reply::Opened {
            id: 1,
            error: None,
            ..
        }
    ));
    assert!(
        replies
            .iter()
            .any(|reply| matches!(reply, Reply::PageText { id: 3, .. }))
    );
    assert!(
        replies
            .iter()
            .any(|reply| matches!(reply, Reply::Tile { id: 2, .. }))
    );
    let founds = replies
        .iter()
        .filter(|reply| matches!(reply, Reply::Found { find: 4, .. }))
        .count();
    assert_eq!(founds, 3, "the whole find ran before the loop ended");
    assert_eq!(
        cursor.position() as usize,
        cursor.get_ref().len(),
        "no stray bytes after the last frame"
    );
}

#[test]
fn a_closed_output_ends_the_loop_quietly() {
    let (mut request_writer, request_reader) = pipe();
    let (reply_writer, reply_reader) = pipe();
    drop(reply_reader);
    let thread =
        std::thread::spawn(move || serve(FakeBackend::default(), request_reader, reply_writer));
    // The Ready frame already fails: nobody is listening.
    let _ = write_request(&mut request_writer, &open(1));
    assert_eq!(thread.join().unwrap(), ServeExit::OutputClosed);
}

/// A backend that returns what a hostile file could make an engine return.
struct Hostile;

impl PdfBackend for Hostile {
    fn open(&mut self, _: &Path, _: Option<&str>) -> Result<DocumentInfo, PdfError> {
        Ok(DocumentInfo {
            pages: vec![PageSize::new(100.0, 100.0); 3],
            title: Some(format!("line\nbreak\t{}", "t".repeat(10_000))),
        })
    }
    fn render(&mut self, _: &TileRequest) -> Result<Tile, PdfError> {
        Ok(Tile {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        })
    }
    fn page_text(&mut self, _: u32) -> Result<PageText, PdfError> {
        Ok(PageText {
            chars: vec![
                mercury_pdf::types::TextChar {
                    ch: 'a',
                    bounds: Rect::new(f32::NAN, 0.0, 1.0, 1.0),
                };
                3
            ],
        })
    }
    fn find_in_page(
        &mut self,
        page: u32,
        _: &str,
        _: FindOptions,
    ) -> Result<Vec<SearchHit>, PdfError> {
        Ok((0..5_000)
            .map(|index| SearchHit {
                page: 99,
                char_start: index,
                char_len: 1,
                rects: vec![
                    Rect::new(0.0, 0.0, 1.0, 1.0),
                    Rect::new(f32::INFINITY, 0.0, 1.0, 1.0),
                ],
            })
            .chain(std::iter::once(SearchHit {
                page,
                char_start: 0,
                char_len: 0,
                rects: Vec::new(),
            }))
            .collect())
    }
    fn links(&mut self, _: u32) -> Result<Vec<Link>, PdfError> {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        Ok(vec![
            Link {
                rect,
                target: LinkTarget::Uri("https://example.com/ok".to_owned()),
            },
            Link {
                rect,
                target: LinkTarget::Uri(format!("https://example.com/{}", "a".repeat(5_000))),
            },
            Link {
                rect,
                target: LinkTarget::Uri("https://example.com/\nwith newline".to_owned()),
            },
            Link {
                rect,
                target: LinkTarget::Page { page: 99, y: None },
            },
            Link {
                rect,
                target: LinkTarget::Page {
                    page: 1,
                    y: Some(f32::NAN),
                },
            },
            Link {
                rect: Rect::new(f32::NAN, 0.0, 1.0, 1.0),
                target: LinkTarget::Uri("https://example.com/nan".to_owned()),
            },
            Link {
                rect: Rect::new(5.0, 5.0, 5.0, 5.0),
                target: LinkTarget::Uri("https://example.com/empty".to_owned()),
            },
        ])
    }
    fn outline(&mut self) -> Result<Vec<OutlineItem>, PdfError> {
        let mut item = OutlineItem {
            title: "leaf".to_owned(),
            target: Some(LinkTarget::Page { page: 0, y: None }),
            children: Vec::new(),
        };
        for depth in 0..40 {
            item = OutlineItem {
                title: format!("level {depth}\n{}", "x".repeat(6_000)),
                target: Some(LinkTarget::Page { page: 7, y: None }),
                children: vec![item],
            };
        }
        Ok(vec![item])
    }
    fn close(&mut self) {}
}

#[test]
fn what_a_backend_returns_is_cleaned_before_it_is_sent() {
    let mut harness = ServeHarness::start(Hostile);
    ready(&mut harness);
    harness.send(&open(1));
    let (reply, _) = harness.recv();
    let Reply::Opened { title, .. } = reply else {
        panic!("expected Opened");
    };
    let title = title.unwrap();
    assert!(title.len() <= 4096, "{}", title.len());
    assert!(title.starts_with("line break t"), "{title:?}");

    harness.send(&Request::PageText { id: 2, page: 0 });
    let (reply, blob) = harness.recv();
    let Reply::PageText { chars, .. } = reply else {
        panic!("expected PageText");
    };
    let text = decode_page_text(&blob, chars).unwrap();
    assert!(
        text.chars.iter().all(|c| c.bounds.is_empty()),
        "NaN boxes are emptied"
    );

    harness.send(&Request::Links { id: 3, page: 0 });
    let (_, blob) = harness.recv();
    let links: Vec<Link> = decode_list(&blob).unwrap();
    assert_eq!(links.len(), 2, "{links:?}");
    assert_eq!(
        links[0].target,
        LinkTarget::Uri("https://example.com/ok".to_owned())
    );
    assert_eq!(links[1].target, LinkTarget::Page { page: 1, y: None });

    harness.send(&Request::Outline { id: 4 });
    let (_, blob) = harness.recv();
    let outline: Vec<OutlineItem> = decode_list(&blob).unwrap();
    let mut depth = 0;
    let mut level = &outline[0];
    loop {
        depth += 1;
        assert!(level.title.len() <= 4096);
        assert!(!level.title.contains('\n'));
        assert_eq!(
            level.target, None,
            "a page that does not exist is no target"
        );
        match level.children.first() {
            Some(child) => level = child,
            None => break,
        }
    }
    assert_eq!(depth, 16, "the outline is cut at the depth limit");

    harness.send(&Request::Find {
        id: 5,
        query: "a".to_owned(),
        options: FindOptions::default(),
        from_page: 2,
    });
    let (reply, blob) = harness.recv();
    assert!(matches!(reply, Reply::Found { page: 2, .. }));
    let hits: Vec<SearchHit> = decode_list(&blob).unwrap();
    assert_eq!(hits.len(), 1_000, "at most 1000 hits a page");
    assert!(hits.iter().all(|hit| hit.page == 2 && hit.rects.len() == 1));
}

#[test]
fn a_tile_of_the_wrong_size_from_the_backend_is_an_error_not_a_frame() {
    let mut harness = ServeHarness::start(Hostile);
    ready(&mut harness);
    harness.send(&open(1));
    harness.recv();
    // Hostile always returns a 1 x 1 tile; the request wants 8 x 8.
    harness.send(&tile_request(2, 0, tile(0, 0)));
    let (reply, blob) = harness.recv();
    assert!(
        matches!(
            reply,
            Reply::Tile {
                error: Some(PdfError::InvalidRequest(_)),
                ..
            }
        ),
        "{reply:?}"
    );
    assert!(blob.is_empty());
}
