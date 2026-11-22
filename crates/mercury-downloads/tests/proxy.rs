// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The download engine (and with it the PDF viewer's fetch of the page's own file) sends its
//! requests through the proxy that Servo uses, and not around it. A loopback "proxy" records what
//! reaches it: an `http://` request arrives with the whole address (the proxy then answers it),
//! an `https://` request as a `CONNECT host:port` (the proxy refuses it, so only the first line
//! is checked), and a host on the bypass list does not reach the proxy at all.

mod support;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use mercury_downloads::{DownloadEvent, DownloadManager, Timeouts};
use mercury_proxy::ProxyPreferences;
use support::{PDF_BODY, TempDir, TestServer, events_until_done, request, terminal};
use url::Url;

/// A proxy on `127.0.0.1:0` that records the first line of every request it gets and answers a
/// forwarded `GET` with a PDF and a `CONNECT` with `403`.
pub struct LoopbackProxy {
    addr: SocketAddr,
    lines: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LoopbackProxy {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the proxy");
        let addr = listener.local_addr().expect("proxy address");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let lines = Arc::clone(&lines);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Ok(stream) = stream {
                        answer(stream, &lines);
                    }
                }
            })
        };
        Self {
            addr,
            lines,
            stop,
            thread: Some(thread),
        }
    }

    pub fn uri(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("proxy log").clone()
    }
}

impl Drop for LoopbackProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn answer(mut stream: TcpStream, lines: &Mutex<Vec<String>>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let first = head.lines().next().unwrap_or_default().to_owned();
    lines.lock().expect("proxy log").push(first.clone());
    let reply = if first.starts_with("CONNECT ") {
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    } else {
        let mut reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            PDF_BODY.len()
        )
        .into_bytes();
        reply.extend_from_slice(PDF_BODY);
        reply
    };
    let _ = stream.write_all(&reply);
}

fn manager_through(proxy: ProxyPreferences) -> (DownloadManager, Receiver<DownloadEvent>) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let manager = DownloadManager::with_proxy(Timeouts::default(), proxy, move |event| {
        let _ = sender.send(event);
    })
    .expect("start the download manager");
    (manager, receiver)
}

#[test]
fn a_pdf_fetch_goes_through_the_proxy() {
    let proxy = LoopbackProxy::start();
    let dir = TempDir::new("proxy-http");
    let (manager, events) = manager_through(ProxyPreferences {
        http_uri: proxy.uri(),
        ..ProxyPreferences::default()
    });

    // The host does not exist: only a proxy can answer.
    let url = Url::parse("http://pdf.example.test/paper.pdf").expect("url");
    let id = manager.start(request(url, &dir.path));
    let events = events_until_done(&events, id);

    match terminal(&events) {
        DownloadEvent::Finished { target_path, .. } => {
            assert_eq!(std::fs::read(target_path).expect("the file"), PDF_BODY);
        }
        other => panic!("expected Finished, got {other:?}"),
    }
    let lines = proxy.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("GET http://pdf.example.test/paper.pdf "),
        "{lines:?}"
    );
}

#[test]
fn an_https_fetch_asks_the_proxy_for_a_tunnel_to_the_host() {
    let proxy = LoopbackProxy::start();
    let dir = TempDir::new("proxy-https");
    let (manager, events) = manager_through(ProxyPreferences {
        https_uri: proxy.uri(),
        ..ProxyPreferences::default()
    });

    let url = Url::parse("https://pdf.example.test/paper.pdf").expect("url");
    let id = manager.start(request(url, &dir.path));
    let events = events_until_done(&events, id);

    assert!(
        matches!(terminal(&events), DownloadEvent::Failed { .. }),
        "the proxy refused the tunnel: {events:?}"
    );
    assert_eq!(
        proxy.lines(),
        vec!["CONNECT pdf.example.test:443 HTTP/1.1".to_owned()]
    );
}

#[test]
fn a_host_on_the_bypass_list_is_fetched_directly() {
    let proxy = LoopbackProxy::start();
    let server = TestServer::start();
    let dir = TempDir::new("proxy-bypass");
    let (manager, events) = manager_through(ProxyPreferences {
        http_uri: proxy.uri(),
        no_proxy: "localhost,127.0.0.0/8,::1".to_owned(),
        ..ProxyPreferences::default()
    });

    let id = manager.start(request(server.url("/doc.pdf"), &dir.path));
    let events = events_until_done(&events, id);

    assert!(
        matches!(terminal(&events), DownloadEvent::Finished { .. }),
        "{events:?}"
    );
    assert_eq!(proxy.lines(), Vec::<String>::new());
}

#[test]
fn an_unusable_proxy_address_means_no_manager_not_a_direct_connection() {
    let result = DownloadManager::with_proxy(
        Timeouts::default(),
        ProxyPreferences {
            http_uri: "http://".to_owned(),
            ..ProxyPreferences::default()
        },
        |_| {},
    );

    assert!(result.is_err());
}
