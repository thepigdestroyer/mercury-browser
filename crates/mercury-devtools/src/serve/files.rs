// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The file routes: path translation and content types, whole files, directory redirects,
//! index files and listings, and single byte ranges.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use super::http::{Request, Responder, reason};
use super::python::{html_escape, is_python_space, python_strip};

/// The size of one read when a byte range is streamed.
const CHUNK: usize = 65_536;

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

/// A range in one of the three single-range forms, as (first, last) with an empty string for
/// an omitted bound; `None` for anything to answer with the whole file.
pub(super) fn parse_range(header: &str) -> Option<(&str, &str)> {
    let spec = header.strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(first) || !digits(last) || (first.is_empty() && last.is_empty()) {
        return None;
    }
    Some((first, last))
}

/// Digits as a number, saturating where Python's integers would simply be large.
pub(super) fn saturating_number(digits: &str) -> u64 {
    digits.parse().unwrap_or(u64::MAX)
}

pub(super) fn send_range(
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
pub(super) fn send_whole(
    responder: &mut Responder<'_>,
    request: &Request,
    path: &Path,
) -> io::Result<()> {
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
    // Whole seconds, as HTTP dates carry them.
    let modified = UNIX_EPOCH
        + Duration::from_secs(
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |elapsed| elapsed.as_secs()),
        );
    // The three HTTP date forms, all in GMT; anything else is ignored.
    if request.header("If-None-Match").is_none()
        && let Some(since) = request
            .header("If-Modified-Since")
            .and_then(|date| httpdate::parse_http_date(python_strip(date)).ok())
        && modified <= since
    {
        return responder.send_head(304, reason(304), &[]);
    }
    responder.send_head(
        200,
        reason(200),
        &[
            ("Content-type", content_type.into()),
            ("Content-Length", metadata.len().to_string()),
            ("Last-Modified", httpdate::fmt_http_date(modified)),
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
pub(super) fn split_url(target: &str) -> (&str, &str, &str) {
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
}
