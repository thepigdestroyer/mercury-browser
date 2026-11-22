// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! HTTP/1.0 plumbing: reading request lines and headers as Python's
//! `http.server` did, and writing status lines, headers and error pages the way it wrote them.

use std::io::{self, BufRead, Write};
use std::net::TcpStream;
use std::time::SystemTime;

use super::python::{html_escape, latin1, python_int};

/// Python's `http.client._MAXLINE`; a longer request or header line is refused.
pub(super) const MAX_LINE: usize = 65_536;
/// Python's `http.client._MAXHEADERS`; the blank line that ends the headers counts too.
const MAX_HEADERS: usize = 100;

/// The server's name in the `Server` header.
const SERVER_VERSION: &str = concat!("mercury-serve-tests/", env!("CARGO_PKG_VERSION"));

/// Reads up to and including a newline, at most `limit` bytes.
pub(super) fn read_line(reader: &mut impl BufRead, limit: usize) -> io::Result<Vec<u8>> {
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

pub(super) struct Request {
    pub(super) command: String,
    pub(super) path: String,
    pub(super) headers: Vec<(String, String)>,
}

impl Request {
    /// The first header of this name (case-insensitive), as Python's `email.message` finds it.
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Where a response goes, and how: HTTP/0.9 requests get the body only.
pub(super) struct Responder<'a> {
    pub(super) stream: &'a TcpStream,
    pub(super) http09: bool,
    pub(super) head: bool,
}

impl Responder<'_> {
    /// The status line, `Server`, `Date`, the given headers, then the two headers every
    /// response carries and the blank line.
    pub(super) fn send_head(
        &mut self,
        code: u16,
        reason: &str,
        headers: &[(&str, String)],
    ) -> io::Result<()> {
        if self.http09 {
            return Ok(());
        }
        let mut text = format!(
            "HTTP/1.0 {code} {reason}\r\nServer: {SERVER_VERSION}\r\nDate: {}\r\n",
            httpdate::fmt_http_date(SystemTime::now())
        );
        for (name, value) in headers {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
        text.push_str("Accept-Ranges: bytes\r\nCache-Control: no-store\r\n\r\n");
        let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
        self.stream.write_all(&bytes)
    }

    pub(super) fn send_body(&mut self, body: &[u8]) -> io::Result<()> {
        if self.head {
            return Ok(());
        }
        self.stream.write_all(body)
    }

    /// An error response, as Python's `send_error` sent it: the message is also the status
    /// line's reason phrase, `Connection: close` and a small HTML page (with `detail` when
    /// given) follow. The page is this server's own; nothing reads it.
    pub(super) fn send_error(
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
pub(super) fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        414 => "Request-URI Too Long",
        416 => "Requested Range Not Satisfiable",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

pub(super) fn parse_version_number(version: &str) -> Option<(i64, i64)> {
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

pub(super) enum HeaderError {
    LineTooLong,
    TooMany,
}

/// Reads the header block as `http.client.parse_headers` does, and keeps the fields its mail
/// parser would find: a line that is not a header ends the fields (the rest of the block is
/// still read), a line starting with a space or tab continues the previous field.
pub(super) fn read_headers(
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
