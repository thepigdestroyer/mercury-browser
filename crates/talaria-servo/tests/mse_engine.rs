// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Runs the four MSE pages of lane med-1b (`tests/pages/mse-{abort,underrun,tracks,quality}.html`) in
//! a real Servo engine on a software rendering context, and requires each page's title to end with
//! `(done)` and to say `0 FAIL`.
//!
//! Ignored: it needs a real engine (a GL context that may hang while the screen is locked), the
//! GStreamer runtime libraries, and the fixtures of `crates/mercury-mse-testkit`. Run it with
//! `cargo test --release -p talaria-servo --test mse_engine -- --ignored --nocapture` from a worktree
//! after `eval "$(scripts/macos/gstreamer.sh env)"`, on an unlocked screen.
//!
//! One test, not one per page: Servo starts once per process, and the pages run one after the other in
//! the same engine (each in a new web view). `MERCURY_DEV_MSE=1` is set for the process before the
//! engine starts: `dom_media_source_enabled` is pinned off otherwise.
//!
//! NOT RUN in round 2a (no window run is made by the lane). If a page cannot play media on a software
//! rendering context, the central window run (`scripts/dev/mse-window-run.sh pages`) is the authority.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use servo::SoftwareRenderingContext;
use talaria_core::{BrowserEvent, TabId};
use talaria_servo::{EventLoopWaker, RuntimeOptions, ServoRuntime};

/// The pages, in the order of the lane brief, and how long each may take.
const PAGES: [(&str, Duration); 4] = [
    ("mse-abort.html", Duration::from_secs(90)),
    ("mse-underrun.html", Duration::from_secs(90)),
    ("mse-tracks.html", Duration::from_secs(60)),
    ("mse-quality.html", Duration::from_secs(90)),
];

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// The file a request path names, or `None`: only `/pages/<file>` and `/mse-fixtures/<path>`, no `..`.
fn resolve(root: &Path, request_path: &str) -> Option<PathBuf> {
    let path = request_path.split(['?', '#']).next()?;
    if path.contains("..") || path.contains('%') || path.contains('\\') {
        return None;
    }
    let (base, rest) = if let Some(rest) = path.strip_prefix("/pages/") {
        (root.join("tests/pages"), rest)
    } else if let Some(rest) = path.strip_prefix("/mse-fixtures/") {
        (root.join("crates/mercury-mse-testkit/fixtures"), rest)
    } else {
        return None;
    };
    let file = base.join(rest);
    file.is_file().then_some(file)
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

fn serve_one(mut stream: TcpStream, root: &Path) {
    let mut request = [0u8; 4096];
    let Ok(length) = stream.read(&mut request) else {
        return;
    };
    let text = String::from_utf8_lossy(&request[..length]);
    let target = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let response = match resolve(root, target)
        .and_then(|file| fs::read(&file).ok().map(|b| (file, b)))
    {
        Some((file, body)) => {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                content_type(&file),
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(&body);
            response
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };
    let _ = stream.write_all(&response);
}

/// Serves the pages and fixtures on an ephemeral loopback port until the process ends.
fn start_server(root: PathBuf) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port is free");
    let port = listener
        .local_addr()
        .expect("a bound socket has an address")
        .port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let root = root.clone();
            thread::spawn(move || serve_one(stream, &root));
        }
    });
    port
}

#[test]
#[ignore = "needs a real engine: a GL context that may hang while the screen is locked"]
fn the_mse_pages_pass_in_a_real_engine() {
    // Before the engine starts: the process reads this once.
    // SAFETY: nothing else runs yet in this process that reads the environment.
    unsafe { std::env::set_var("MERCURY_DEV_MSE", "1") };

    let root = repository_root();
    let port = start_server(root.clone());

    let runtime = ServoRuntime::new(
        EventLoopWaker::new(|| {}),
        RuntimeOptions {
            gpu_canvas: false,
            ..RuntimeOptions::default()
        },
    );
    let (sender, events) = mpsc::channel();
    let mut failures = Vec::new();

    for (index, (page, limit)) in PAGES.iter().enumerate() {
        let rendering_context = Rc::new(
            SoftwareRenderingContext::new(PhysicalSize::new(640, 480))
                .expect("a software rendering context can be created"),
        );
        let url = format!("http://127.0.0.1:{port}/pages/{page}");
        let tab_id = TabId::new(index as u64 + 1);
        let view = runtime
            .create_web_view(rendering_context, &url, 1.0, tab_id, sender.clone())
            .expect("the web view is created");
        view.show();

        let started = Instant::now();
        let mut title = String::new();
        while started.elapsed() < *limit && !title.ends_with("(done)") {
            runtime.spin_event_loop();
            while let Ok(event) = events.try_recv() {
                if event.tab_id() == tab_id
                    && let BrowserEvent::PageTitleChanged { title: Some(new) } = event.into_event()
                {
                    title = new;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        println!("{page}: {title:?} after {} s", started.elapsed().as_secs());
        if !(title.ends_with("(done)") && title.contains(" 0 FAIL")) {
            failures.push(format!("{page}: {title:?}"));
        }
        drop(view);
        runtime.spin_event_loop();
    }

    assert!(
        failures.is_empty(),
        "pages that did not pass: {failures:#?}"
    );
}

#[test]
fn only_the_pages_and_the_fixtures_are_served() {
    let root = repository_root();
    assert!(resolve(&root, "/pages/mse-abort.html").is_some());
    assert!(resolve(&root, "/pages/mse-abort.html?fixtures=/x").is_some());
    assert!(resolve(&root, "/mse-fixtures/fmp4/video-h264-low.mp4").is_some());
    assert!(resolve(&root, "/pages/../../Cargo.toml").is_none());
    assert!(resolve(&root, "/pages/%2e%2e/Cargo.toml").is_none());
    assert!(resolve(&root, "/Cargo.toml").is_none());
    assert!(resolve(&root, "/pages/").is_none());
}
