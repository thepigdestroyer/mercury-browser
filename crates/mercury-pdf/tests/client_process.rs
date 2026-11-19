// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The client against a real helper process (`pdf-fake-worker`): the happy path, a helper that is
//! killed in the middle of a request, one that hangs, one that breaks the protocol, one that
//! panics, the cleared environment, and that no process is left behind.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use mercury_pdf::client::{ClientConfig, ClientEvent, LIBRARY_VARIABLE, PdfClient, WorkerLaunch};
use mercury_pdf::error::PdfError;
use mercury_pdf::fake::expected_tile;
use mercury_pdf::types::{FindOptions, Rotation, TileRequest};

const WAIT: Duration = Duration::from_secs(20);

fn launch(args: &[&str]) -> WorkerLaunch {
    WorkerLaunch {
        program: PathBuf::from(env!("CARGO_BIN_EXE_pdf-fake-worker")),
        args: args.iter().map(OsString::from).collect(),
        library: None,
    }
}

fn collect() -> (
    Box<dyn Fn(ClientEvent) + Send + Sync>,
    Receiver<ClientEvent>,
) {
    let (sender, receiver) = mpsc::channel();
    let sender = Mutex::new(sender);
    (
        Box::new(move |event| {
            let _ = sender.lock().unwrap().send(event);
        }),
        receiver,
    )
}

fn start(args: &[&str]) -> (PdfClient, Receiver<ClientEvent>) {
    let (events, receiver) = collect();
    let client = PdfClient::start(launch(args), events).expect("the helper starts");
    (client, receiver)
}

fn next(receiver: &Receiver<ClientEvent>) -> ClientEvent {
    receiver.recv_timeout(WAIT).expect("an event in time")
}

fn tile(page: u32, x: u32) -> TileRequest {
    TileRequest {
        page,
        scale: 1.0,
        rotation: Rotation::Deg0,
        x,
        y: 0,
        width: 16,
        height: 16,
    }
}

/// Whether a process with this id exists (a zombie counts: that is what reaping is for).
fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn kill_hard(pid: u32) {
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("kill runs");
    assert!(status.success());
}

/// Waits (briefly) until the process is gone.
fn wait_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn open_a_tile_text_and_a_find_through_a_real_process() {
    let (client, events) = start(&["--pages=4"]);
    assert!(client.process_id().is_some());
    client.open(Path::new("/fake/file.pdf"), None);
    let ClientEvent::Opened(Ok(info)) = next(&events) else {
        panic!("expected Opened");
    };
    assert_eq!(info.page_count(), 4);
    let request = tile(2, 32);
    let id = client.request_tile(request, 1);
    let ClientEvent::Tile {
        id: got,
        request: got_request,
        result,
    } = next(&events)
    else {
        panic!("expected a tile");
    };
    assert_eq!((got, got_request), (id, request));
    assert_eq!(result.unwrap(), expected_tile(&request));

    client.request_page_text(1);
    let ClientEvent::PageText { page, result } = next(&events) else {
        panic!("expected text");
    };
    assert_eq!(page, 1);
    assert!(result.unwrap().text().starts_with("Page 2: the quick"));

    let find = client.find("dog", FindOptions::default(), 3);
    let mut pages = Vec::new();
    loop {
        let ClientEvent::Found {
            find: got,
            page,
            hits,
            finished,
        } = next(&events)
        else {
            panic!("expected Found");
        };
        assert_eq!(got, find);
        assert_eq!(hits.len(), 1);
        pages.push(page);
        if finished {
            break;
        }
    }
    assert_eq!(pages, [3, 0, 1, 2]);
}

#[test]
fn the_helper_runs_with_an_empty_environment() {
    // Without a library: no variable at all.
    let (client, events) = start(&["--report-env"]);
    client.open(Path::new("/fake/file.pdf"), None);
    let ClientEvent::Opened(Ok(info)) = next(&events) else {
        panic!("expected Opened");
    };
    assert_eq!(info.title, None, "no variable at all, so an empty title");
    drop(client);

    // With one: only that variable.
    let (events_sink, receiver) = collect();
    let mut with_library = launch(&["--report-env"]);
    with_library.library = Some(PathBuf::from("/somewhere/libpdfium.dylib"));
    let client = PdfClient::start(with_library, events_sink).unwrap();
    client.open(Path::new("/fake/file.pdf"), None);
    let ClientEvent::Opened(Ok(info)) = next(&receiver) else {
        panic!("expected Opened");
    };
    assert_eq!(info.title.as_deref(), Some(LIBRARY_VARIABLE));
}

#[test]
fn a_helper_killed_in_the_middle_of_a_request_is_one_loss_and_then_silence() {
    let (client, events) = start(&["--render-delay-ms=30000"]);
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
    let pid = client.process_id().expect("a process");
    client.request_tile(tile(0, 0), 1);
    client.request_tile(tile(0, 16), 1);
    client.request_tile(tile(0, 32), 1);
    std::thread::sleep(Duration::from_millis(300));
    kill_hard(pid);
    let ClientEvent::WorkerLost(error) = next(&events) else {
        panic!("expected the loss");
    };
    assert!(matches!(error, PdfError::WorkerLost(_)), "{error:?}");
    assert!(
        error.detail().contains("signal"),
        "the loss says how the helper ended: {error:?}"
    );
    assert!(client.has_ended());
    // Nothing more, and calls after the loss do nothing.
    client.request_page_text(0);
    client.request_tile(tile(1, 0), 1);
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(
        events.recv_timeout(Duration::from_millis(500)),
        Err(RecvTimeoutError::Timeout)
    ));
    assert!(wait_gone(pid), "the killed helper was reaped");
}

#[test]
fn a_helper_that_hangs_is_ended_after_the_time_limit() {
    let (events, receiver) = collect();
    let client = PdfClient::start_with_config(
        launch(&["--render-delay-ms=60000"]),
        events,
        ClientConfig {
            open_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_millis(400),
        },
    )
    .unwrap();
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(next(&receiver), ClientEvent::Opened(Ok(_))));
    let pid = client.process_id().unwrap();
    let asked = Instant::now();
    client.request_tile(tile(0, 0), 1);
    assert!(matches!(
        next(&receiver),
        ClientEvent::WorkerLost(PdfError::Timeout)
    ));
    let waited = asked.elapsed();
    assert!(
        waited >= Duration::from_millis(350) && waited < Duration::from_secs(10),
        "{waited:?}"
    );
    assert!(matches!(
        receiver.recv_timeout(Duration::from_millis(400)),
        Err(RecvTimeoutError::Timeout)
    ));
    assert!(wait_gone(pid));
}

#[test]
fn a_helper_that_never_says_it_is_ready_hits_the_open_limit() {
    let (events, receiver) = collect();
    let _client = PdfClient::start_with_config(
        launch(&["--silent"]),
        events,
        ClientConfig {
            open_timeout: Duration::from_millis(400),
            request_timeout: Duration::from_secs(30),
        },
    )
    .unwrap();
    assert!(matches!(
        next(&receiver),
        ClientEvent::WorkerLost(PdfError::Timeout)
    ));
}

#[test]
fn a_slow_but_steady_helper_is_not_cut_off() {
    // Each tile takes 150 ms; the limit is 400 ms from the last answer, and seven tiles take
    // longer than that in all.
    let (events, receiver) = collect();
    let client = PdfClient::start_with_config(
        launch(&["--render-delay-ms=150"]),
        events,
        ClientConfig {
            open_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_millis(400),
        },
    )
    .unwrap();
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(next(&receiver), ClientEvent::Opened(Ok(_))));
    for index in 0..7 {
        client.request_tile(tile(0, index * 16), 1);
    }
    for _ in 0..7 {
        assert!(matches!(
            next(&receiver),
            ClientEvent::Tile { result: Ok(_), .. }
        ));
    }
}

#[test]
fn a_helper_that_breaks_the_protocol_is_ended_and_reported_once() {
    let (client, events) = start(&["--garbage"]);
    let pid = client.process_id().unwrap();
    let ClientEvent::WorkerLost(error) = next(&events) else {
        panic!("expected the loss");
    };
    assert!(matches!(error, PdfError::WorkerLost(_)));
    assert!(error.detail().contains("bad frame"), "{error:?}");
    assert!(matches!(
        events.recv_timeout(Duration::from_millis(400)),
        Err(RecvTimeoutError::Timeout)
    ));
    assert!(wait_gone(pid));
}

#[test]
fn a_helper_that_panics_answers_the_request_and_ends_with_status_70() {
    let (client, events) = start(&["--panic-after=0"]);
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
    client.request_tile(tile(0, 0), 1);
    assert!(matches!(
        next(&events),
        ClientEvent::Tile {
            result: Err(PdfError::WorkerLost(_)),
            ..
        }
    ));
    let ClientEvent::WorkerLost(error) = next(&events) else {
        panic!("expected the loss");
    };
    assert!(error.detail().contains("70"), "{error:?}");
}

#[test]
fn a_password_protected_document_through_the_process() {
    let (client, events) = start(&["--password=sesame"]);
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(
        next(&events),
        ClientEvent::Opened(Err(PdfError::PasswordRequired))
    ));
    client.open(Path::new("/fake/file.pdf"), Some("sesame"));
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
}

#[test]
fn dropping_the_client_leaves_no_process_behind() {
    // Idle helper.
    let (client, events) = start(&[]);
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
    let idle = client.process_id().unwrap();
    assert!(process_exists(idle));
    drop(client);
    assert!(wait_gone(idle), "an idle helper is gone after the drop");

    // A helper in the middle of a long render.
    let (client, events) = start(&["--render-delay-ms=60000"]);
    client.open(Path::new("/fake/file.pdf"), None);
    assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
    let busy = client.process_id().unwrap();
    client.request_tile(tile(0, 0), 1);
    std::thread::sleep(Duration::from_millis(200));
    let dropped = Instant::now();
    drop(client);
    assert!(
        dropped.elapsed() < Duration::from_secs(5),
        "the drop does not wait for the render"
    );
    assert!(wait_gone(busy), "a busy helper is gone after the drop");
    // And no event arrives after the drop.
    assert!(
        events
            .try_iter()
            .all(|event| !matches!(event, ClientEvent::WorkerLost(_)))
    );
}

#[test]
fn a_program_that_does_not_exist_is_an_error_not_a_loss() {
    let (events, _receiver) = collect();
    let result = PdfClient::start(
        WorkerLaunch {
            program: PathBuf::from("/nonexistent/mercury-pdf-worker"),
            args: Vec::new(),
            library: None,
        },
        events,
    );
    assert!(matches!(result, Err(PdfError::Io(_))));
}

#[test]
fn the_launch_for_the_browser_itself_is_its_own_executable_with_the_worker_flag() {
    let launch = WorkerLaunch::current_exe(Some(PathBuf::from("/lib/libpdfium.dylib"))).unwrap();
    assert_eq!(launch.program, std::env::current_exe().unwrap());
    assert_eq!(launch.args, [OsString::from("--pdf-worker")]);
    assert_eq!(launch.library, Some(PathBuf::from("/lib/libpdfium.dylib")));
}
