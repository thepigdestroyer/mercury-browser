// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The loopback server for the repository's `tests/` directory, which replaces
//! `scripts/serve-tests.py` with the same behaviour.
//!
//! It answers as Python's `http.server.SimpleHTTPRequestHandler`, which the Python script was
//! built on, did (HTTP/1.0, one request per connection, a thread per connection, files with
//! the same content types, directory redirects, index files and listings, `If-Modified-Since`,
//! the same status lines and headers; only the HTML of error pages and listings is this
//! server's own), plus the script's own additions:
//!
//! - single byte ranges (`Range: bytes=a-b`, `a-` and `-n`) answered with 206 Partial
//!   Content, or 416 when unsatisfiable; any other `Range` is ignored and the whole file sent;
//! - `/private`, which asks for HTTP Basic authentication (realm `Mercury test realm`) and
//!   accepts only `alice` / `secret`;
//! - `Accept-Ranges: bytes` and `Cache-Control: no-store` on every response.
//!
//! The work is split by job: `cli` reads the command line, `http` reads requests and
//! writes responses, `files` answers the file routes and `python` holds the Python-style
//! text helpers the others share. This module accepts connections and routes each request.

mod cli;
mod files;
mod http;
mod python;

pub use cli::{Command, DESCRIPTION, UsageError, help_text, parse_arguments};
pub use files::{guess_type, translate_path};

use std::fs;
use std::io::{self, BufReader};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use files::{parse_range, send_range, send_whole};
use http::{
    HeaderError, MAX_LINE, Request, Responder, parse_version_number, read_headers, read_line,
    reason,
};
use python::{is_python_space, latin1, python_repr, python_strip};

/// The route that asks for HTTP Basic authentication.
pub const PRIVATE_PATH: &str = "/private";
/// The realm `/private` names in its challenge.
pub const PRIVATE_REALM: &str = "Mercury test realm";
/// `Authorization` for the only accepted credentials, `alice` / `secret`.
pub const PRIVATE_CREDENTIALS: &str = "Basic YWxpY2U6c2VjcmV0";

/// The repository's `tests/` directory, as the crate was built from.
pub fn default_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

/// A bound server: 127.0.0.1 on the requested or an ephemeral port.
pub struct Server {
    listener: TcpListener,
    root: Arc<PathBuf>,
}

impl Server {
    /// Binds 127.0.0.1:`port` (0: any free port) to serve `root`.
    pub fn bind(root: &Path, port: u16) -> io::Result<Server> {
        let root = fs::canonicalize(root)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        Ok(Server {
            listener,
            root: Arc::new(root),
        })
    }

    /// The port the server listens on.
    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or(0)
    }

    /// Answers connections until the process ends, each on its own thread.
    pub fn run(self) {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let root = Arc::clone(&self.root);
                    let spawned = std::thread::Builder::new()
                        .name("serve-tests connection".into())
                        .spawn(move || handle_connection(stream, &root));
                    if spawned.is_err() {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                // Python's server ignores a failed accept and goes on (for example when out
                // of file descriptors); the pause keeps that from spinning.
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    /// Runs the server on a background thread and returns its port (for tests).
    pub fn spawn(self) -> u16 {
        let port = self.port();
        std::thread::spawn(move || self.run());
        port
    }
}

fn handle_connection(stream: TcpStream, root: &Path) {
    // A client that goes away mid-response only ends its own connection. Python printed a
    // traceback to standard error then; nothing here does.
    let _ = handle_request(&stream, root);
    let _ = stream.shutdown(Shutdown::Write);
}

/// Reads one request from the connection and answers it.
fn handle_request(stream: &TcpStream, root: &Path) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut responder = Responder {
        stream,
        http09: false,
        head: false,
    };
    let raw = read_line(&mut reader, MAX_LINE + 1)?;
    if raw.len() > MAX_LINE {
        return responder.send_error(414, None, None);
    }
    if raw.is_empty() {
        return Ok(());
    }
    let line = latin1(&raw);
    let line = line.trim_end_matches(['\r', '\n']);
    let words: Vec<&str> = line
        .split(is_python_space)
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return Ok(());
    }
    // Python's default request version is HTTP/0.9 until a valid version is read, so the
    // errors below go out as a bare body.
    responder.http09 = true;
    if words.len() >= 3 {
        let version = words[words.len() - 1];
        let Some(number) = parse_version_number(version) else {
            let message = format!("Bad request version ({})", python_repr(version));
            return responder.send_error(400, Some(&message), None);
        };
        if number >= (2, 0) {
            let base = &version["HTTP/".len()..];
            let message = format!("Invalid HTTP version ({base})");
            return responder.send_error(505, Some(&message), None);
        }
        responder.http09 = version == "HTTP/0.9";
    }
    if !(2..=3).contains(&words.len()) {
        let message = format!("Bad request syntax ({})", python_repr(line));
        return responder.send_error(400, Some(&message), None);
    }
    let command = words[0];
    if words.len() == 2 && command != "GET" {
        let message = format!("Bad HTTP/0.9 request type ({})", python_repr(command));
        return responder.send_error(400, Some(&message), None);
    }
    let mut path = words[1].to_owned();
    if path.starts_with("//") {
        path = format!("/{}", path.trim_start_matches('/'));
    }
    responder.head = command == "HEAD";
    let headers = match read_headers(&mut reader)? {
        Ok(headers) => headers,
        Err(HeaderError::LineTooLong) => {
            return responder.send_error(
                431,
                Some("Line too long"),
                Some("got more than 65536 bytes when reading header line"),
            );
        }
        Err(HeaderError::TooMany) => {
            return responder.send_error(
                431,
                Some("Too many headers"),
                Some("got more than 100 headers"),
            );
        }
    };
    let request = Request {
        command: command.to_owned(),
        path,
        headers,
    };
    match request.command.as_str() {
        "GET" | "HEAD" => respond(&mut responder, &request, root),
        other => {
            let message = format!("Unsupported method ({})", python_repr(other));
            responder.send_error(501, Some(&message), None)
        }
    }
}

/// Routes a `GET` or `HEAD`: `/private`, a byte range of a file, or the whole file.
fn respond(responder: &mut Responder<'_>, request: &Request, root: &Path) -> io::Result<()> {
    if request.path.split('?').next() == Some(PRIVATE_PATH) {
        return send_private(responder, request);
    }
    let path = translate_path(root, &request.path);
    if let Some(header) = request.header("Range")
        && !path.is_dir()
        && path.is_file()
        && let Some((first, last)) = parse_range(python_strip(header))
    {
        return send_range(responder, &path, first, last);
    }
    send_whole(responder, request, &path)
}

/// The `/private` route: 200 with the accepted credentials, otherwise 401 and a challenge.
fn send_private(responder: &mut Responder<'_>, request: &Request) -> io::Result<()> {
    let signed_in = request.header("Authorization") == Some(PRIVATE_CREDENTIALS);
    let (code, body) = if signed_in {
        (200, "Signed in.\n")
    } else {
        (401, "Sign-in required.\n")
    };
    let mut headers = Vec::new();
    if !signed_in {
        headers.push((
            "WWW-Authenticate",
            format!("Basic realm=\"{PRIVATE_REALM}\""),
        ));
    }
    headers.push(("Content-Type", "text/plain; charset=utf-8".into()));
    headers.push(("Content-Length", body.len().to_string()));
    responder.send_head(code, reason(code), &headers)?;
    responder.send_body(body.as_bytes())
}
