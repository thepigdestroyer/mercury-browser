// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The routes of the MSE window run (`docs/developer-guide/mse-testing.md`), all read-only
//! except `/__report`:
//!
//! - `/wpt/<path>`, and `/resources/`, `/common/`, `/interfaces/`, `/media-source/` at the root
//!   (web-platform-tests pages name their helpers by absolute path): the sparse checkout of
//!   `scripts/dev/fetch-mse-tools.sh`, with `resources/testharnessreport.js` replaced by the
//!   report script and `.any.html` / `.window.html` pages made from `.any.js` / `.window.js`
//!   files (see `wpt`);
//! - `/mse-players/<file>`: the fetched dash.js, Shaka Player and hls.js builds;
//! - `/mse-dash/<set>/...` and `/mse-hls/<set>/...`: the generated streams (see `streams`);
//! - `/__mse/reporter.js`, and the same script inserted into a page of `tests/` opened with
//!   `?__mse_run=<token>`;
//! - `POST /__report`: the body goes to the report sink (`Server::with_report_sink`), which is
//!   how `mse-run` reads what a page or a WPT file reports.
//!
//! Every response has the headers and range support of the other file routes. A path with `..`
//! (plain or percent-encoded) can only name something below the route's own directory.

use std::io::{self, BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use percent_encoding::percent_decode_str;

use super::downloads::url_path;
use super::files::{
    parse_range, saturating_number, send_range, send_whole, split_url, translate_path,
};
use super::http::{Request, Responder, reason};
use super::python::python_strip;
use super::streams::{Body, Family, Streams};
use super::wpt;

/// Where `/__report` bodies go.
pub type ReportSink = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// The most a `/__report` body may hold (a WPT file with thousands of subtests is well below).
const MAX_REPORT_BYTES: u64 = 8 * 1024 * 1024;

/// The report script `serve-tests` inserts into an instrumented page.
pub const REPORTER_JS: &str = include_str!("../../resources/mse-reporter.js");
/// Mercury's `testharnessreport.js` for the WPT pages.
pub const WPT_REPORT_JS: &str = include_str!("../../resources/mse-wpt-report.js");

/// The query parameter that makes a page report (`mse-run` adds it).
pub const TOKEN_PARAMETER: &str = "__mse_run";

/// The directories of the WPT checkout that are also served at the root of the server.
const WPT_ROOT_ALIASES: [&str; 4] = ["/resources/", "/common/", "/interfaces/", "/media-source/"];

/// The directory `scripts/dev/fetch-mse-tools.sh` fills: `$MSE_TOOLS_DIR`, else the cache
/// directory the disk-guard plugin keeps.
pub fn mse_tools_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MSE_TOOLS_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Caches/mercury-browser/ci-tools/mse")
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache"))
            .join("mercury-browser/ci-tools/mse")
    }
}

/// The MSE routes of a server.
pub struct Routes {
    tools: PathBuf,
    sink: Option<ReportSink>,
    streams: Streams,
}

impl Routes {
    pub fn new(tools: PathBuf) -> Routes {
        Routes {
            tools,
            sink: None,
            streams: Streams::default(),
        }
    }

    pub fn with_sink(mut self, sink: ReportSink) -> Routes {
        self.sink = Some(sink);
        self
    }

    pub fn with_tools_dir(mut self, tools: PathBuf) -> Routes {
        self.tools = tools;
        self
    }

    /// Answers a `GET` or `HEAD` this module owns; `None` for any other request. `root` is the
    /// directory of the ordinary file routes (`tests/`).
    pub(super) fn get(
        &self,
        responder: &mut Responder<'_>,
        request: &Request,
        root: &Path,
    ) -> Option<io::Result<()>> {
        let path = url_path(&request.path);
        if path == "/__mse/reporter.js" {
            return Some(send_bytes(
                responder,
                request,
                "text/javascript",
                REPORTER_JS.as_bytes(),
            ));
        }
        if let Some(rest) = path.strip_prefix("/mse-players/") {
            return Some(self.send_tool_file(
                responder,
                request,
                &self.tools.join("players"),
                rest,
            ));
        }
        if let Some(rest) = path.strip_prefix("/mse-dash/") {
            return Some(self.send_generated(responder, request, Family::Dash, rest));
        }
        if let Some(rest) = path.strip_prefix("/mse-hls/") {
            return Some(self.send_generated(responder, request, Family::Hls, rest));
        }
        if let Some(rest) = path.strip_prefix("/wpt/") {
            return Some(self.send_wpt(responder, request, rest));
        }
        if WPT_ROOT_ALIASES.iter().any(|alias| path.starts_with(alias)) {
            return Some(self.send_wpt(responder, request, path));
        }
        if token_of(&request.path).is_some()
            && path.ends_with(".html")
            && !path.starts_with("/mse-fixtures/")
        {
            let file = translate_path(root, &request.path);
            if file.is_file() {
                return Some(send_instrumented_html(responder, request, &file));
            }
        }
        None
    }

    /// Answers `POST /__report`; `None` for any other request (nothing has been read).
    pub(super) fn post(
        &self,
        responder: &mut Responder<'_>,
        reader: &mut impl BufRead,
        request: &Request,
    ) -> Option<io::Result<()>> {
        if url_path(&request.path) != "/__report" {
            return None;
        }
        let length = request
            .header("Content-Length")
            .and_then(|value| python_strip(value).parse::<u64>().ok());
        let Some(length) = length else {
            return Some(responder.send_error(400, Some("Content-Length required"), None));
        };
        if length > MAX_REPORT_BYTES {
            return Some(responder.send_error(413, None, None));
        }
        let mut body = Vec::new();
        if let Err(error) = reader.by_ref().take(length).read_to_end(&mut body) {
            return Some(Err(error));
        }
        if let Some(sink) = &self.sink {
            sink(&body);
        }
        Some(responder.send_head(204, reason(204), &[("Content-Length", "0".into())]))
    }

    fn send_tool_file(
        &self,
        responder: &mut Responder<'_>,
        request: &Request,
        dir: &Path,
        rest: &str,
    ) -> io::Result<()> {
        if !dir.is_dir() {
            return missing_tools(responder, dir);
        }
        send_file(
            responder,
            request,
            &translate_path(dir, &format!("/{rest}")),
        )
    }

    fn send_generated(
        &self,
        responder: &mut Responder<'_>,
        request: &Request,
        family: Family,
        rest: &str,
    ) -> io::Result<()> {
        let rest = percent_decode_str(rest).decode_utf8_lossy();
        match self.streams.answer(family, &rest) {
            Some(Body {
                content_type,
                bytes,
            }) => send_bytes(responder, request, content_type, &bytes),
            None => responder.send_error(404, Some("File not found"), None),
        }
    }

    fn send_wpt(
        &self,
        responder: &mut Responder<'_>,
        request: &Request,
        rest: &str,
    ) -> io::Result<()> {
        let wpt_root = self.tools.join("wpt");
        if !wpt_root.is_dir() {
            return missing_tools(responder, &wpt_root);
        }
        let file = translate_path(&wpt_root, rest);
        if file == wpt_root.join("resources/testharnessreport.js") {
            return send_bytes(
                responder,
                request,
                "text/javascript",
                WPT_REPORT_JS.as_bytes(),
            );
        }
        if !file.exists()
            && let Some(relative) = file.strip_prefix(&wpt_root).ok().and_then(Path::to_str)
            && let Some((source, kind)) = wpt::source_of_wrapper(relative)
            && let Ok(js) = std::fs::read_to_string(wpt_root.join(&source))
        {
            let meta = wpt::parse_meta(&js);
            let name = source.rsplit('/').next().unwrap_or(&source);
            let page = wpt::wrapper_html(name, &meta, kind);
            return send_bytes(responder, request, "text/html", page.as_bytes());
        }
        send_file(responder, request, &file)
    }
}

/// The value of `__mse_run` in a request target's query.
pub fn token_of(target: &str) -> Option<String> {
    let (_, query, _) = split_url(target);
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == TOKEN_PARAMETER && !value.is_empty())
            .then(|| percent_decode_str(value).decode_utf8_lossy().into_owned())
    })
}

fn missing_tools(responder: &mut Responder<'_>, dir: &Path) -> io::Result<()> {
    let message = format!(
        "{} is missing: run scripts/dev/fetch-mse-tools.sh",
        dir.display()
    );
    responder.send_error(404, Some(&message), None)
}

/// A file with the range support of the other file routes.
fn send_file(responder: &mut Responder<'_>, request: &Request, file: &Path) -> io::Result<()> {
    if let Some(header) = request.header("Range")
        && file.is_file()
        && let Some((first, last)) = parse_range(python_strip(header))
    {
        send_range(responder, file, first, last)
    } else {
        send_whole(responder, request, file)
    }
}

/// `page` with the reporter script as the first element of its `<head>`.
pub fn insert_reporter(page: &str) -> String {
    let tag = "<script src=\"/__mse/reporter.js\"></script>";
    let lower = page.to_ascii_lowercase();
    for opener in ["<head", "<html"] {
        if let Some(start) = lower.find(opener)
            && let Some(end) = lower[start..].find('>')
            && lower[start + opener.len()..start + end]
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || c == '>' || c == '/')
        {
            let at = start + end + 1;
            return format!("{}{tag}{}", &page[..at], &page[at..]);
        }
    }
    format!("{tag}{page}")
}

fn send_instrumented_html(
    responder: &mut Responder<'_>,
    request: &Request,
    file: &Path,
) -> io::Result<()> {
    let Ok(bytes) = std::fs::read(file) else {
        return responder.send_error(404, Some("File not found"), None);
    };
    let page = insert_reporter(&String::from_utf8_lossy(&bytes));
    send_bytes(
        responder,
        request,
        "text/html; charset=utf-8",
        page.as_bytes(),
    )
}

/// The bounds of a single range over `size` bytes, as the file routes answer one; `None` when
/// it cannot be satisfied.
fn range_bounds(size: u64, first: &str, last: &str) -> Option<(u64, u64)> {
    let (start, end) = if first.is_empty() {
        (
            size.saturating_sub(saturating_number(last)),
            size.checked_sub(1)?,
        )
    } else {
        let start = saturating_number(first);
        let final_byte = size.checked_sub(1)?;
        let end = if last.is_empty() {
            final_byte
        } else {
            saturating_number(last).min(final_byte)
        };
        (start, end)
    };
    (size != 0 && start < size && start <= end).then_some((start, end))
}

/// Bytes held in memory, answered whole or as one range.
pub(super) fn send_bytes(
    responder: &mut Responder<'_>,
    request: &Request,
    content_type: &str,
    bytes: &[u8],
) -> io::Result<()> {
    let size = bytes.len() as u64;
    if let Some(header) = request.header("Range")
        && let Some((first, last)) = parse_range(python_strip(header))
    {
        let Some((start, end)) = range_bounds(size, first, last) else {
            return responder.send_head(
                416,
                reason(416),
                &[
                    ("Content-Range", format!("bytes */{size}")),
                    ("Content-Length", "0".into()),
                ],
            );
        };
        responder.send_head(
            206,
            reason(206),
            &[
                ("Content-Type", content_type.into()),
                ("Content-Range", format!("bytes {start}-{end}/{size}")),
                ("Content-Length", (end - start + 1).to_string()),
            ],
        )?;
        return responder.send_body(&bytes[start as usize..=end as usize]);
    }
    responder.send_head(
        200,
        reason(200),
        &[
            ("Content-Type", content_type.into()),
            ("Content-Length", size.to_string()),
        ],
    )?;
    responder.send_body(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_read_from_the_query() {
        assert_eq!(token_of("/pages/a.html?__mse_run=t1"), Some("t1".into()));
        assert_eq!(
            token_of("/pages/a.html?x=1&__mse_run=a%20b#frag"),
            Some("a b".into())
        );
        assert_eq!(token_of("/pages/a.html?__mse_run="), None);
        assert_eq!(token_of("/pages/a.html?x=__mse_run=1"), None);
        assert_eq!(token_of("/pages/a.html"), None);
    }

    #[test]
    fn the_reporter_goes_first_in_the_head() {
        assert_eq!(
            insert_reporter("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<title>x</title>"),
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head><script src=\"/__mse/reporter.js\"></script>\n<title>x</title>"
        );
        assert_eq!(
            insert_reporter("<HEAD data-x=1><title>x</title>"),
            "<HEAD data-x=1><script src=\"/__mse/reporter.js\"></script><title>x</title>"
        );
        assert_eq!(
            insert_reporter("<html><body>x"),
            "<html><script src=\"/__mse/reporter.js\"></script><body>x"
        );
        assert_eq!(
            insert_reporter("<header>x</header>"),
            "<script src=\"/__mse/reporter.js\"></script><header>x</header>"
        );
        assert_eq!(
            insert_reporter("plain"),
            "<script src=\"/__mse/reporter.js\"></script>plain"
        );
    }

    #[test]
    fn range_bounds_follow_the_file_routes() {
        assert_eq!(range_bounds(10, "0", "3"), Some((0, 3)));
        assert_eq!(range_bounds(10, "4", ""), Some((4, 9)));
        assert_eq!(range_bounds(10, "", "3"), Some((7, 9)));
        assert_eq!(range_bounds(10, "8", "99"), Some((8, 9)));
        assert_eq!(range_bounds(10, "10", ""), None);
        assert_eq!(range_bounds(10, "5", "2"), None);
        assert_eq!(range_bounds(0, "0", ""), None);
    }
}
