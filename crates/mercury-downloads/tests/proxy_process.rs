// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `DownloadManager::new`, the constructor the browser uses, takes the proxy of the process
//! (`mercury_proxy::install`, called once at start with the same inputs as Servo's preferences).
//! One test in a file of its own, because the process proxy can be set only once.

use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use mercury_downloads::{DownloadEvent, DownloadManager, DownloadRequest};
use mercury_proxy::ProxyPreferences;

#[test]
fn the_browsers_manager_sends_its_requests_to_the_proxy_of_the_process() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the proxy");
    mercury_proxy::install_preferences(ProxyPreferences {
        http_uri: format!("http://{}", listener.local_addr().expect("address")),
        ..ProxyPreferences::default()
    });
    let (sender, events) = mpsc::channel();
    let manager = DownloadManager::new(move |event| {
        let _ = sender.send(event);
    })
    .expect("start the download manager");
    let folder = std::env::temp_dir().join(format!("mercury-proxy-process-{}", std::process::id()));

    manager.start(DownloadRequest {
        url: "http://pdf.example.test/a.pdf".parse().expect("url"),
        referrer: None,
        user_agent: "MercuryTest/1.0".to_owned(),
        cookie_header: None,
        destination_dir: folder.clone(),
        suggested_name: None,
        resume: None,
        max_bytes: None,
    });

    // The proxy only has to be reached: it never answers, and the transfer is dropped with the
    // manager.
    listener.set_nonblocking(true).expect("non-blocking");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let reached = loop {
        if listener.accept().is_ok() {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(manager);
    let _ = std::fs::remove_dir_all(&folder);
    assert!(
        reached,
        "the request never reached the proxy of the process"
    );
    assert!(
        !events
            .try_iter()
            .any(|event| matches!(event, DownloadEvent::Finished { .. })),
        "nothing can finish without an answer"
    );
}
