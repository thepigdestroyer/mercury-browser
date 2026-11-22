// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A small HTTP/1.1 server on `127.0.0.1:0` for the fetch tests: it records what each request
//! looked like, answers as a handler says, and counts connections and hang-ups. A proxy is the
//! same server with a handler that reads the request line. Nothing here leaves the loopback
//! interface.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mercury_view_source::{CookieLookup, FetchOptions, ProxyConfig, ViewSourceUrl};
use url::Url;

/// One request as the server saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    /// `GET /path?query HTTP/1.1`, or `CONNECT host:port HTTP/1.1`.
    pub request_line: String,
    /// Header names in lower case, values as sent, in order.
    pub headers: Vec<(String, String)>,
}

impl Seen {
    /// The request target, the second word of the request line.
    pub fn target(&self) -> &str {
        self.request_line.split(' ').nth(1).unwrap_or_default()
    }

    /// The path of the target, without the query.
    pub fn path(&self) -> &str {
        self.target().split('?').next().unwrap_or_default()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn header_names(&self) -> Vec<&str> {
        self.headers.iter().map(|(name, _)| name.as_str()).collect()
    }
}

/// What a server does with a request.
pub enum Reply {
    /// Writes these bytes and closes the connection.
    Bytes(Vec<u8>),
    /// Never answers; the connection stays open until the server is dropped.
    Stall,
    /// Writes these bytes, then keeps the connection open and waits for the client to hang up.
    ThenHold(Vec<u8>),
}

enum Mode {
    Http(Arc<dyn Fn(&Seen) -> Reply + Send + Sync>),
    Raw(Vec<u8>),
}

struct State {
    seen: Mutex<Vec<Seen>>,
    /// What a raw server read first on each connection.
    raw_first: Mutex<Vec<Vec<u8>>>,
    connections: AtomicUsize,
    hangups: AtomicUsize,
    stop: AtomicBool,
}

/// A server that stops when it is dropped.
pub struct Server {
    pub addr: SocketAddr,
    state: Arc<State>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Self {
        Self::spawn(Mode::Http(Arc::new(handler)))
    }

    /// A server that is not HTTP: it reads what the client sends first (a TLS hello, say), writes
    /// `bytes` and closes.
    pub fn raw(bytes: &[u8]) -> Self {
        Self::spawn(Mode::Raw(bytes.to_vec()))
    }

    fn spawn(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let addr = listener.local_addr().expect("local address");
        let state = Arc::new(State {
            seen: Mutex::new(Vec::new()),
            raw_first: Mutex::new(Vec::new()),
            connections: AtomicUsize::new(0),
            hangups: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        let mode = Arc::new(mode);
        let accepting = Arc::clone(&state);
        let thread = thread::spawn(move || {
            let mut workers: Vec<JoinHandle<()>> = Vec::new();
            while !accepting.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // A socket accepted from a non-blocking listener is non-blocking too on
                        // macOS; the workers use timeouts instead.
                        let _ = stream.set_nonblocking(false);
                        accepting.connections.fetch_add(1, Ordering::SeqCst);
                        let state = Arc::clone(&accepting);
                        let mode = Arc::clone(&mode);
                        workers.push(thread::spawn(move || match &*mode {
                            Mode::Http(handler) => serve(stream, &state, handler.as_ref()),
                            Mode::Raw(bytes) => serve_raw(stream, &state, bytes),
                        }));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Self {
            addr,
            state,
            thread: Some(thread),
        }
    }

    /// A server that answers every request the same way.
    pub fn always(reply: impl Fn() -> Reply + Send + Sync + 'static) -> Self {
        Self::start(move |_| reply())
    }

    /// `http://127.0.0.1:<port><path>`
    pub fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://127.0.0.1:{}{path}", self.addr.port())).expect("test URL")
    }

    /// `view-source:http://127.0.0.1:<port><path>`
    pub fn source(&self, path: &str) -> ViewSourceUrl {
        ViewSourceUrl::for_page(&self.url(path)).expect("a source address")
    }

    /// `http://127.0.0.1:<port>`: how a proxy is written in a preference.
    pub fn proxy_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.state
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// What a raw server read first on each connection (a TLS hello starts with 0x16).
    pub fn first_bytes(&self) -> Vec<Vec<u8>> {
        self.state
            .raw_first
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Connections the server accepted, whether or not a request followed.
    pub fn connections(&self) -> usize {
        self.state.connections.load(Ordering::SeqCst)
    }

    /// Connections that a `ThenHold` reply saw the client close.
    pub fn hangups(&self) -> usize {
        self.state.hangups.load(Ordering::SeqCst)
    }

    /// Waits until `count` hang-ups were seen; `false` if that takes longer than `limit`. It waits
    /// with `tokio::time::sleep`, not by blocking the thread: the test's runtime has to keep
    /// running so that the connection of a dropped request is actually closed.
    pub async fn wait_for_hangups(&self, count: usize, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if self.hangups() >= count {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.hangups() >= count
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_raw(mut stream: TcpStream, state: &State, bytes: &[u8]) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut first = [0_u8; 4096];
    if let Ok(count) = stream.read(&mut first) {
        state
            .raw_first
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(first[..count].to_vec());
    }
    let _ = stream.write_all(bytes);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

fn serve(mut stream: TcpStream, state: &State, handler: &(dyn Fn(&Seen) -> Reply + Send + Sync)) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(seen) = read_request(&mut stream, state) else {
        return;
    };
    state
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(seen.clone());
    match handler(&seen) {
        Reply::Bytes(bytes) => {
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Stall => wait_for_stop(&mut stream, state, false),
        Reply::ThenHold(bytes) => {
            if stream.write_all(&bytes).is_ok() {
                let _ = stream.flush();
                wait_for_stop(&mut stream, state, true);
            }
        }
    }
}

/// Holds the connection until the server stops. With `count_hangup`, a client that closes its
/// side (or resets it) ends the wait and is counted.
fn wait_for_stop(stream: &mut TcpStream, state: &State, count_hangup: bool) {
    let mut buffer = [0_u8; 256];
    while !state.stop.load(Ordering::SeqCst) {
        match stream.read(&mut buffer) {
            Ok(0) => {
                if count_hangup {
                    state.hangups.fetch_add(1, Ordering::SeqCst);
                }
                return;
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                if count_hangup {
                    state.hangups.fetch_add(1, Ordering::SeqCst);
                }
                return;
            }
        }
    }
}

/// Reads the request line and the headers (there is never a body).
fn read_request(stream: &mut TcpStream, state: &State) -> Option<Seen> {
    let mut head = Vec::new();
    let mut buffer = [0_u8; 1024];
    let deadline = Instant::now() + Duration::from_secs(10);
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if state.stop.load(Ordering::SeqCst) || Instant::now() > deadline || head.len() > 65_536 {
            return None;
        }
        match stream.read(&mut buffer) {
            Ok(0) => return None,
            Ok(count) => head.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next()?.to_owned();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    Some(Seen {
        request_line,
        headers,
    })
}

/// A response with `Content-Length` and `Connection: close` (unless the headers give their own).
pub fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("HTTP/1.1 {status}\r\n").into_bytes();
    let has = |name: &str| {
        headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(name))
    };
    for (name, value) in headers {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    if !has("content-length") && !has("transfer-encoding") {
        bytes.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    }
    bytes.extend_from_slice(b"Connection: close\r\n\r\n");
    bytes.extend_from_slice(body);
    bytes
}

/// `200 OK` with a `Content-Type`.
pub fn ok(content_type: &str, body: &[u8]) -> Reply {
    Reply::Bytes(response("200 OK", &[("Content-Type", content_type)], body))
}

pub fn html(body: &str) -> Reply {
    ok("text/html; charset=utf-8", body.as_bytes())
}

/// A redirect with the given status code and `Location`.
pub fn redirect(code: u16, location: &str) -> Reply {
    Reply::Bytes(response(
        &format!("{code} Redirect"),
        &[("Location", location)],
        b"moved",
    ))
}

/// A body in chunks of the sizes given, as `Transfer-Encoding: chunked`.
pub fn chunked(content_type: &str, parts: &[&[u8]]) -> Reply {
    let mut bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    for part in parts {
        bytes.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
        bytes.extend_from_slice(part);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"0\r\n\r\n");
    Reply::Bytes(bytes)
}

/// The options of a test: generous time limits, no cookies, no proxy.
pub fn options() -> FetchOptions {
    FetchOptions {
        user_agent: "MercuryTest/1.0".to_owned(),
        cookies: None,
        sec_gpc: false,
        proxy: ProxyConfig::default(),
        accept_language: String::new(),
        policy: None,
        connect_timeout: Duration::from_secs(5),
        total_timeout: Duration::from_secs(20),
    }
}

/// Options with a cookie lookup that records every address it is asked for and answers with the
/// text `answer` makes of it.
pub fn options_with_cookies(
    answer: impl Fn(&Url) -> Option<String> + Send + Sync + 'static,
) -> (FetchOptions, Arc<Mutex<Vec<Url>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let lookup: CookieLookup = Arc::new(move |url: &Url, _earlier: &[Url]| {
        log.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.clone());
        answer(url)
    });
    let mut options = options();
    options.cookies = Some(lookup);
    (options, asked)
}

/// A loopback port nothing listens on.
pub fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    listener.local_addr().expect("local address").port()
}
