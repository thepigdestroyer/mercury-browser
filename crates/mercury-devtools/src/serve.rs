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

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::num::IntErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::httpdate;

/// The route that asks for HTTP Basic authentication.
pub const PRIVATE_PATH: &str = "/private";
/// The realm `/private` names in its challenge.
pub const PRIVATE_REALM: &str = "Mercury test realm";
/// `Authorization` for the only accepted credentials, `alice` / `secret`.
pub const PRIVATE_CREDENTIALS: &str = "Basic YWxpY2U6c2VjcmV0";

/// Python's `http.client._MAXLINE`; a longer request or header line is refused.
const MAX_LINE: usize = 65_536;
/// Python's `http.client._MAXHEADERS`; the blank line that ends the headers counts too.
const MAX_HEADERS: usize = 100;
const CHUNK: usize = 65_536;

/// The server's name in the `Server` header.
const SERVER_VERSION: &str = concat!("mercury-serve-tests/", env!("CARGO_PKG_VERSION"));

/// The description `--help` prints, the first line of the old script's documentation.
pub const DESCRIPTION: &str =
    "Serve the repository's tests/ directory on the loopback interface, for the media checks.";
const PROGRAM: &str = "serve-tests";
const USAGE: &str = "usage: serve-tests [-h] [--port PORT]";

/// The repository's `tests/` directory, as the crate was built from.
pub fn default_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Print the help text on standard output and exit 0.
    Help,
    /// Serve on this port (0: any free port). Not checked against 0..=65535 here: like the
    /// Python script, an out-of-range port fails when binding, with exit status 1.
    Serve { port: i64 },
}

/// A command-line error: the message for standard error and the exit status.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
    pub status: i32,
}

/// The `--help` text.
pub fn help_text() -> String {
    format!(
        "{USAGE}\n\n{DESCRIPTION}\n\noptional arguments:\n  -h, --help   show this help message and exit\n  --port PORT  port to bind (default: any free port)\n"
    )
}

fn usage_error(detail: &str) -> UsageError {
    UsageError {
        message: format!("{USAGE}\n{PROGRAM}: error: {detail}\n"),
        status: 2,
    }
}

fn python_int(text: &str) -> Option<i64> {
    match text.trim().parse::<i64>() {
        Ok(value) => Some(value),
        Err(error) => match error.kind() {
            IntErrorKind::PosOverflow => Some(i64::MAX),
            IntErrorKind::NegOverflow => Some(i64::MIN),
            _ => None,
        },
    }
}

/// Whether `argument` names an option by itself or by an unambiguous prefix (argparse
/// accepts `--po 8000` for `--port 8000`).
fn long_option(argument: &str) -> Option<&'static str> {
    if argument.len() < 3 || !argument.starts_with("--") {
        return None;
    }
    ["--help", "--port"]
        .into_iter()
        .find(|option| option.starts_with(argument))
}

fn looks_like_negative_number(argument: &str) -> bool {
    argument
        .strip_prefix('-')
        .is_some_and(|rest| !rest.is_empty() && rest.parse::<f64>().is_ok())
}

/// Reads the command line (without the program name) as the Python script's `argparse`
/// parser did: `[-h] [--port PORT]`, with `--port=N` and unambiguous prefixes.
pub fn parse_arguments(arguments: &[String]) -> Result<Command, UsageError> {
    let mut port = 0;
    let mut unrecognised = Vec::new();
    let mut only_positional = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        index += 1;
        if only_positional
            || !argument.starts_with('-')
            || argument == "-"
            || looks_like_negative_number(argument)
        {
            unrecognised.push(argument.to_owned());
            continue;
        }
        if argument == "--" {
            only_positional = true;
            continue;
        }
        let (name, inline_value) = match argument.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (argument, None),
        };
        let option = if name == "-h" {
            Some("--help")
        } else {
            long_option(name)
        };
        match option {
            Some("--help") => {
                if inline_value.is_some() {
                    return Err(usage_error(&format!(
                        "argument -h/--help: ignored explicit argument '{}'",
                        inline_value.unwrap_or_default()
                    )));
                }
                return Ok(Command::Help);
            }
            Some(_) => {
                let value = match inline_value {
                    Some(value) => value,
                    None => match arguments.get(index) {
                        Some(next)
                            if !next.starts_with('-')
                                || next == "-"
                                || looks_like_negative_number(next) =>
                        {
                            index += 1;
                            next.as_str()
                        }
                        _ => return Err(usage_error("argument --port: expected one argument")),
                    },
                };
                port = python_int(value).ok_or_else(|| {
                    usage_error(&format!("argument --port: invalid int value: '{value}'"))
                })?;
            }
            None => unrecognised.push(argument.to_owned()),
        }
    }
    if !unrecognised.is_empty() {
        return Err(usage_error(&format!(
            "unrecognized arguments: {}",
            unrecognised.join(" ")
        )));
    }
    Ok(Command::Serve { port })
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

/// Reads up to and including a newline, at most `limit` bytes.
fn read_line(reader: &mut impl BufRead, limit: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    while line.len() < limit {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            break;
        }
        let room = (limit - line.len()).min(available.len());
        if let Some(newline) = available[..room].iter().position(|&byte| byte == b'\n') {
            line.extend_from_slice(&available[..=newline]);
            reader.consume(newline + 1);
            break;
        }
        line.extend_from_slice(&available[..room]);
        reader.consume(room);
    }
    Ok(line)
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// Python's `str.isspace()` for the characters ISO-8859-1 can hold.
fn is_python_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}'
    )
}

fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_space)
}

/// Python's `repr()` of a string decoded from ISO-8859-1.
fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c < ' ' || ('\u{7f}'..='\u{a0}').contains(&c) || c == '\u{ad}' => {
                out.push_str(&format!("\\x{:02x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `html.escape(text, quote=False)`.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `urllib.parse.unquote`: percent-decoding to UTF-8, invalid sequences replaced.
fn unquote(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// `urllib.parse.quote(text)`: everything but letters, digits, `_.-~` and `/` encoded.
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

struct Request {
    command: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    /// The first header of this name (case-insensitive), as Python's `email.message` finds it.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Where a response goes, and how: HTTP/0.9 requests get the body only.
struct Responder<'a> {
    stream: &'a TcpStream,
    http09: bool,
    head: bool,
}

impl Responder<'_> {
    /// The status line, `Server`, `Date`, the given headers, then the two headers every
    /// response carries and the blank line.
    fn send_head(&mut self, code: u16, reason: &str, headers: &[(&str, String)]) -> io::Result<()> {
        if self.http09 {
            return Ok(());
        }
        let mut text = format!(
            "HTTP/1.0 {code} {reason}\r\nServer: {SERVER_VERSION}\r\nDate: {}\r\n",
            httpdate::format(now())
        );
        for (name, value) in headers {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
        text.push_str("Accept-Ranges: bytes\r\nCache-Control: no-store\r\n\r\n");
        let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
        self.stream.write_all(&bytes)
    }

    fn send_body(&mut self, body: &[u8]) -> io::Result<()> {
        if self.head {
            return Ok(());
        }
        self.stream.write_all(body)
    }

    /// An error response, as Python's `send_error` sent it: the message is also the status
    /// line's reason phrase, `Connection: close` and a small HTML page (with `detail` when
    /// given) follow. The page is this server's own; nothing reads it.
    fn send_error(
        &mut self,
        code: u16,
        message: Option<&str>,
        detail: Option<&str>,
    ) -> io::Result<()> {
        let message = message.unwrap_or(reason(code));
        let detail = detail
            .map(|detail| format!("<p>{}</p>\n", html_escape(detail)))
            .unwrap_or_default();
        let body = format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error {code}</title>\n</head>\n<body>\n<h1>Error {code}</h1>\n<p>{}</p>\n{detail}</body>\n</html>\n",
            html_escape(message)
        );
        self.send_head(
            code,
            message,
            &[
                ("Connection", "close".into()),
                ("Content-Type", "text/html;charset=utf-8".into()),
                ("Content-Length", body.len().to_string()),
            ],
        )?;
        self.send_body(body.as_bytes())
    }
}

/// The reason phrase of each status this server sends.
fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        206 => "Partial Content",
        301 => "Moved Permanently",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        414 => "Request-URI Too Long",
        416 => "Requested Range Not Satisfiable",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn parse_version_number(version: &str) -> Option<(i64, i64)> {
    let base = version.strip_prefix("HTTP/")?;
    let mut parts = base.split('.');
    let major = python_int(parts.next()?)?;
    let minor = python_int(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor))
}

/// The header-name characters Python's mail parser accepts (`[\041-\071\073-\176]`).
fn is_header_name(name: &str) -> bool {
    name.chars()
        .all(|c| ('\u{21}'..='\u{39}').contains(&c) || ('\u{3b}'..='\u{7e}').contains(&c))
}

enum HeaderError {
    LineTooLong,
    TooMany,
}

/// Reads the header block as `http.client.parse_headers` does, and keeps the fields its mail
/// parser would find: a line that is not a header ends the fields (the rest of the block is
/// still read), a line starting with a space or tab continues the previous field.
fn read_headers(
    reader: &mut impl BufRead,
) -> io::Result<Result<Vec<(String, String)>, HeaderError>> {
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut lines = 0;
    let mut ended = false;
    loop {
        let line = read_line(reader, MAX_LINE + 1)?;
        if line.len() > MAX_LINE {
            return Ok(Err(HeaderError::LineTooLong));
        }
        lines += 1;
        if lines > MAX_HEADERS {
            return Ok(Err(HeaderError::TooMany));
        }
        if line == b"\r\n" || line == b"\n" || line.is_empty() {
            break;
        }
        if ended {
            continue;
        }
        let text = latin1(&line);
        if text.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push_str(&text);
            }
            continue;
        }
        if text.starts_with("From ") {
            continue;
        }
        match text.split_once(':') {
            Some((name, value)) if is_header_name(name) => {
                headers.push((
                    name.to_owned(),
                    value.trim_start_matches([' ', '\t']).to_owned(),
                ));
            }
            _ => ended = true,
        }
    }
    for (_, value) in &mut headers {
        let trimmed = value.trim_end_matches(['\r', '\n']).len();
        value.truncate(trimmed);
    }
    Ok(Ok(headers))
}

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

/// A range in one of the three single-range forms, as (first, last) with an empty string for
/// an omitted bound; `None` for anything to answer with the whole file.
fn parse_range(header: &str) -> Option<(&str, &str)> {
    let spec = header.strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(first) || !digits(last) || (first.is_empty() && last.is_empty()) {
        return None;
    }
    Some((first, last))
}

/// Digits as a number, saturating where Python's integers would simply be large.
fn saturating_number(digits: &str) -> u64 {
    digits.parse().unwrap_or(u64::MAX)
}

fn send_range(
    responder: &mut Responder<'_>,
    path: &Path,
    first: &str,
    last: &str,
) -> io::Result<()> {
    let size = fs::metadata(path)?.len();
    let (start, end) = if first.is_empty() {
        (
            size.saturating_sub(saturating_number(last)),
            size.checked_sub(1),
        )
    } else {
        let start = saturating_number(first);
        let end = if last.is_empty() {
            size.checked_sub(1)
        } else {
            size.checked_sub(1)
                .map(|final_byte| saturating_number(last).min(final_byte))
        };
        (start, end)
    };
    let end = match end {
        Some(end) if size != 0 && start < size && start <= end => end,
        _ => {
            return responder.send_head(
                416,
                reason(416),
                &[
                    ("Content-Range", format!("bytes */{size}")),
                    ("Content-Length", "0".into()),
                ],
            );
        }
    };
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let length = end - start + 1;
    responder.send_head(
        206,
        reason(206),
        &[
            ("Content-Type", guess_type(path).into()),
            ("Content-Range", format!("bytes {start}-{end}/{size}")),
            ("Content-Length", length.to_string()),
        ],
    )?;
    if responder.head {
        return Ok(());
    }
    let mut remaining = length;
    let mut buffer = vec![0; CHUNK];
    while remaining > 0 {
        let want = remaining.min(CHUNK as u64) as usize;
        let read = match file.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        responder.stream.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    Ok(())
}

/// Python's `SimpleHTTPRequestHandler.send_head` and `copyfile` for a whole file, a
/// directory redirect, an index file or a listing.
fn send_whole(responder: &mut Responder<'_>, request: &Request, path: &Path) -> io::Result<()> {
    let mut path = path.to_path_buf();
    if path.is_dir() {
        let (before, query, fragment) = split_url(&request.path);
        if !before.ends_with('/') {
            let mut location = format!("{before}/");
            if !query.is_empty() {
                location.push('?');
                location.push_str(query);
            }
            if !fragment.is_empty() {
                location.push('#');
                location.push_str(fragment);
            }
            return responder.send_head(
                301,
                reason(301),
                &[("Location", location), ("Content-Length", "0".into())],
            );
        }
        match ["index.html", "index.htm"]
            .into_iter()
            .map(|index| path.join(index))
            .find(|index| index.exists())
        {
            Some(index) => path = index,
            None => return list_directory(responder, request, &path),
        }
    }
    let content_type = guess_type(&path);
    if path.as_os_str().as_encoded_bytes().ends_with(b"/") {
        return responder.send_error(404, Some("File not found"), None);
    }
    // Opening a directory succeeds here, where Python's open() fails.
    let opened = File::open(&path).and_then(|file| {
        let metadata = file.metadata()?;
        if metadata.is_dir() {
            return Err(io::Error::other("is a directory"));
        }
        Ok((file, metadata))
    });
    let Ok((mut file, metadata)) = opened else {
        return responder.send_error(404, Some("File not found"), None);
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    if request.header("If-None-Match").is_none()
        && let Some(since) = request
            .header("If-Modified-Since")
            .and_then(httpdate::parse)
        && since.is_utc()
        && modified <= since.unix_seconds
    {
        return responder.send_head(304, reason(304), &[]);
    }
    responder.send_head(
        200,
        reason(200),
        &[
            ("Content-type", content_type.into()),
            ("Content-Length", metadata.len().to_string()),
            ("Last-Modified", httpdate::format(modified)),
        ],
    )?;
    if responder.head {
        return Ok(());
    }
    io::copy(&mut file, &mut responder.stream)?;
    Ok(())
}

/// The path, query and fragment of a request path, as `urllib.parse.urlsplit` separates them
/// (the fragment first, then the query).
fn split_url(target: &str) -> (&str, &str, &str) {
    let (rest, fragment) = target.split_once('#').unwrap_or((target, ""));
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    (path, query, fragment)
}

fn list_directory(responder: &mut Responder<'_>, request: &Request, path: &Path) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(path) else {
        return responder.send_error(404, Some("No permission to list directory"), None);
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    let title = format!(
        "Directory listing for {}",
        html_escape(&unquote(&request.path))
    );
    let mut lines = vec![
        "<!DOCTYPE html>".to_owned(),
        "<html lang=\"en\">".to_owned(),
        "<meta charset=\"utf-8\">".to_owned(),
        format!("<title>{title}</title>"),
        format!("<h1>{title}</h1>"),
        "<ul>".to_owned(),
    ];
    for name in names {
        let full = path.join(&name);
        let mut display = name.clone();
        let mut link = name.clone();
        if full.is_dir() {
            display.push('/');
            link.push('/');
        }
        if full.is_symlink() {
            display = format!("{name}@");
        }
        lines.push(format!(
            "<li><a href=\"{}\">{}</a></li>",
            utf8_percent_encode(&link, QUOTE_SET),
            html_escape(&display)
        ));
    }
    lines.push("</ul>\n</html>\n".to_owned());
    let body = lines.join("\n");
    responder.send_head(
        200,
        reason(200),
        &[
            ("Content-type", "text/html; charset=utf-8".into()),
            ("Content-Length", body.len().to_string()),
        ],
    )?;
    responder.send_body(body.as_bytes())
}

/// Python's `SimpleHTTPRequestHandler.translate_path`: the query and fragment dropped, the
/// path percent-decoded and normalised, and every `.`, `..` and empty component ignored, so
/// that nothing outside `root` can be named. A trailing slash is kept.
pub fn translate_path(root: &Path, target: &str) -> PathBuf {
    let target = target.split('?').next().unwrap_or_default();
    let target = target.split('#').next().unwrap_or_default();
    let trailing_slash = target.trim_end_matches(is_python_space).ends_with('/');
    let decoded = unquote(target);
    let mut words: Vec<&str> = Vec::new();
    let absolute = decoded.starts_with('/');
    for word in decoded.split('/') {
        match word {
            "" | "." => {}
            ".." => {
                if words.last().is_some_and(|last| *last != "..") {
                    words.pop();
                } else if !absolute {
                    words.push(word);
                }
            }
            word => words.push(word),
        }
    }
    let mut path = root.to_path_buf();
    for word in words {
        if word != "." && word != ".." {
            path.push(word);
        }
    }
    if trailing_slash {
        let mut text = OsString::from(path);
        text.push("/");
        path = PathBuf::from(text);
    }
    path
}

/// The `Content-Type` Python 3.9 chose on macOS for a file name (its compression map, then
/// `mimetypes` with the system's `/etc/apache2/mime.types`), for the types a test page uses.
/// Anything else is `application/octet-stream`.
pub fn guess_type(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let trimmed = name.trim_start_matches('.');
    let Some((_, extension)) = trimmed.rsplit_once('.') else {
        return "application/octet-stream";
    };
    if extension == "Z" {
        return "application/octet-stream";
    }
    match extension.to_lowercase().as_str() {
        "gz" => "application/gzip",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        "html" | "htm" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "m4a" => "audio/mp4a-latm",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "ogv" => "video/ogg",
        "wav" => "audio/x-wav",
        "flac" => "audio/x-flac",
        "aac" => "audio/x-aac",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(list: &[&str]) -> Vec<String> {
        list.iter().map(|argument| (*argument).to_owned()).collect()
    }

    #[test]
    fn command_line_like_argparse() {
        let serve = |port| Ok(Command::Serve { port });
        assert_eq!(parse_arguments(&[]), serve(0));
        assert_eq!(
            parse_arguments(&arguments(&["--port", "8765"])),
            serve(8765)
        );
        assert_eq!(parse_arguments(&arguments(&["--port=8765"])), serve(8765));
        assert_eq!(
            parse_arguments(&arguments(&["--po", "1", "--port", "2"])),
            serve(2)
        );
        assert_eq!(parse_arguments(&arguments(&["--port", "-1"])), serve(-1));
        assert_eq!(parse_arguments(&arguments(&["-h"])), Ok(Command::Help));
        assert_eq!(
            parse_arguments(&arguments(&["x", "--help"])),
            Ok(Command::Help)
        );
        for (list, detail) in [
            (&["--port"][..], "argument --port: expected one argument"),
            (
                &["--port", "--help"][..],
                "argument --port: expected one argument",
            ),
            (
                &["--port", "x"][..],
                "argument --port: invalid int value: 'x'",
            ),
            (
                &["--port", "80", "extra"][..],
                "unrecognized arguments: extra",
            ),
            (&["-p", "80"][..], "unrecognized arguments: -p 80"),
            (&["--", "--port"][..], "unrecognized arguments: --port"),
            (&["--verbose"][..], "unrecognized arguments: --verbose"),
        ] {
            let error = parse_arguments(&arguments(list)).expect_err(detail);
            assert_eq!(error.status, 2);
            assert_eq!(
                error.message,
                format!("{USAGE}\nserve-tests: error: {detail}\n")
            );
        }
    }

    #[test]
    fn help_names_the_option() {
        let help = help_text();
        assert!(help.starts_with("usage: serve-tests [-h] [--port PORT]\n"));
        assert!(help.contains("--port PORT  port to bind (default: any free port)"));
    }

    #[test]
    fn paths_stay_inside_the_root() {
        let root = Path::new("/srv/tests");
        let cases = [
            ("/", "/srv/tests/"),
            ("/pages/media.html", "/srv/tests/pages/media.html"),
            ("/pages/media.html?x=1#y", "/srv/tests/pages/media.html"),
            ("/pages/", "/srv/tests/pages/"),
            ("/../../etc/passwd", "/srv/tests/etc/passwd"),
            ("/%2e%2e/%2E%2E/etc/passwd", "/srv/tests/etc/passwd"),
            ("/pages/../media/tone.wav", "/srv/tests/media/tone.wav"),
            ("/a/./b//c", "/srv/tests/a/b/c"),
            ("/a%2Fb", "/srv/tests/a/b"),
            ("/hello%20world.txt", "/srv/tests/hello world.txt"),
            ("relative/../../x", "/srv/tests/x"),
            ("/a/..", "/srv/tests"),
        ];
        for (target, expected) in cases {
            assert_eq!(
                translate_path(root, target),
                PathBuf::from(expected),
                "{target}"
            );
        }
    }

    #[test]
    fn content_types_match_python_on_macos() {
        for (name, expected) in [
            ("media.html", "text/html"),
            ("MEDIA.HTML", "text/html"),
            ("clip-vp9-opus.webm", "video/webm"),
            ("clip-h264-aac.mp4", "video/mp4"),
            ("tone-opus.ogg", "audio/ogg"),
            ("tone.mp3", "audio/mpeg"),
            ("tone.wav", "audio/x-wav"),
            ("README.md", "application/octet-stream"),
            ("helper.js", "text/javascript"),
            ("bundle.tar.gz", "application/gzip"),
            ("archive.Z", "application/octet-stream"),
            (".hidden", "application/octet-stream"),
            ("noextension", "application/octet-stream"),
        ] {
            assert_eq!(guess_type(Path::new(name)), expected, "{name}");
        }
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99"), Some(("0", "99")));
        assert_eq!(parse_range("bytes=5-"), Some(("5", "")));
        assert_eq!(parse_range("bytes=-5"), Some(("", "5")));
        assert_eq!(parse_range("bytes=-"), None);
        assert_eq!(parse_range("bytes=0-1,4-5"), None);
        assert_eq!(parse_range("items=0-1"), None);
        assert_eq!(parse_range("bytes=a-b"), None);
        assert_eq!(parse_range("bytes = 0-1"), None);
    }

    #[test]
    fn repr_like_python() {
        assert_eq!(python_repr("HTTP/x"), "'HTTP/x'");
        assert_eq!(python_repr("it's"), "\"it's\"");
        assert_eq!(python_repr("a'\""), "'a\\'\"'");
        assert_eq!(python_repr("\u{1}\u{e9}\\"), "'\\x01\u{e9}\\\\'");
    }
}
