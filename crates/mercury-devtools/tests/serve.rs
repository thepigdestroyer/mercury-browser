// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The test server over loopback: every route and behaviour the Python server had.

use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::time::Duration;

use mercury_devtools::serve::Server;

struct Response {
    raw: Vec<u8>,
    status_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn header_names(&self) -> Vec<&str> {
        self.headers.iter().map(|(name, _)| name.as_str()).collect()
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn exchange(port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    stream.write_all(request).expect("send");
    stream.shutdown(Shutdown::Write).expect("shutdown");
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .expect("the server closes the connection");
    raw
}

fn parse(raw: Vec<u8>) -> Response {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header block");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default().to_owned();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(": ").expect("a header");
            (name.to_owned(), value.to_owned())
        })
        .collect();
    Response {
        raw,
        status_line,
        headers,
        body,
    }
}

fn get(port: u16, method: &str, path: &str, headers: &[&str]) -> Response {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for header in headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    parse(exchange(port, request.as_bytes()))
}

const PAGE: &str = "<!doctype html><title>page</title><p>Hello</p>\n";

/// A root like `tests/`: pages, media, a directory with an index file and one without, and a
/// secret beside the root that must stay out of reach.
fn fixture() -> (tempfile::TempDir, u16) {
    let dir = tempfile::tempdir().expect("temporary directory");
    let root = dir.path().join("tests");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("media")).unwrap();
    fs::create_dir_all(root.join("indexed")).unwrap();
    fs::create_dir_all(root.join("pages/nested dir")).unwrap();
    fs::write(root.join("pages/page.html"), PAGE).unwrap();
    fs::write(root.join("pages/b&<.txt"), "x").unwrap();
    fs::write(root.join("pages/Zed.js"), "1;").unwrap();
    fs::write(root.join("pages/hello world.txt"), "spaces\n").unwrap();
    fs::write(root.join("indexed/index.html"), "index page\n").unwrap();
    let clip: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(root.join("media/clip.webm"), &clip).unwrap();
    fs::write(root.join("media/tone.wav"), b"RIFF0123WAVE").unwrap();
    fs::write(root.join("media/empty.mp4"), b"").unwrap();
    fs::write(dir.path().join("secret.txt"), "secret").unwrap();
    let port = Server::bind(&root, 0).expect("bind").spawn();
    (dir, port)
}

fn clip() -> Vec<u8> {
    (0..200_000u32).map(|i| (i % 251) as u8).collect()
}

fn assert_common_headers(response: &Response) {
    let names = response.header_names();
    assert_eq!(names.first(), Some(&"Server"), "{names:?}");
    assert_eq!(names.get(1), Some(&"Date"));
    assert_eq!(names[names.len() - 2..], ["Accept-Ranges", "Cache-Control"]);
    assert_eq!(response.header("Accept-Ranges"), Some("bytes"));
    assert_eq!(response.header("Cache-Control"), Some("no-store"));
    let date = response.header("Date").expect("Date");
    assert!(httpdate::parse_http_date(date).is_ok(), "{date}");
}

#[test]
fn serves_a_file_whole() {
    let (dir, port) = fixture();
    let response = get(port, "GET", "/pages/page.html", &[]);
    assert_eq!(response.status_line, "HTTP/1.0 200 OK");
    assert_common_headers(&response);
    assert_eq!(response.header("Content-type"), Some("text/html"));
    assert_eq!(
        response.header("Content-Length"),
        Some(PAGE.len().to_string().as_str())
    );
    let modified = fs::metadata(dir.path().join("tests/pages/page.html"))
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(
        response.header("Last-Modified"),
        Some(
            httpdate::fmt_http_date(std::time::UNIX_EPOCH + Duration::from_secs(modified)).as_str()
        )
    );
    assert_eq!(response.text(), PAGE);
    assert_eq!(
        response.header_names(),
        [
            "Server",
            "Date",
            "Content-type",
            "Content-Length",
            "Last-Modified",
            "Accept-Ranges",
            "Cache-Control"
        ]
    );

    let media = get(port, "GET", "/media/clip.webm?t=1#x", &[]);
    assert_eq!(media.header("Content-type"), Some("video/webm"));
    assert_eq!(media.body, clip());
    assert_eq!(
        get(port, "GET", "/media/tone.wav", &[]).header("Content-type"),
        Some("audio/x-wav")
    );
    assert_eq!(
        get(port, "GET", "/pages/Zed.js", &[]).header("Content-type"),
        Some("text/javascript")
    );
    assert_eq!(
        get(port, "GET", "/pages/hello%20world.txt", &[]).text(),
        "spaces\n"
    );
    assert_eq!(get(port, "GET", "//pages//page.html", &[]).text(), PAGE);
}

#[test]
fn head_sends_the_headers_only() {
    let (_dir, port) = fixture();
    let response = get(port, "HEAD", "/media/clip.webm", &[]);
    assert_eq!(response.status_line, "HTTP/1.0 200 OK");
    assert_eq!(response.header("Content-Length"), Some("200000"));
    assert!(response.body.is_empty());
}

#[test]
fn answers_single_byte_ranges() {
    let (_dir, port) = fixture();
    let clip = clip();
    let size = clip.len();
    let cases: [(&str, usize, usize); 6] = [
        ("bytes=0-99", 0, 99),
        ("bytes=100-", 100, size - 1),
        ("bytes=-500", size - 500, size - 1),
        ("bytes=-999999", 0, size - 1),
        ("bytes=199990-300000", 199_990, size - 1),
        ("  bytes=5-5\t", 5, 5),
    ];
    for (range, start, end) in cases {
        let header = format!("Range: {range}");
        let response = get(port, "GET", "/media/clip.webm", &[&header]);
        assert_eq!(
            response.status_line, "HTTP/1.0 206 Partial Content",
            "{range}"
        );
        assert_common_headers(&response);
        assert_eq!(
            response.header_names()[2..5],
            ["Content-Type", "Content-Range", "Content-Length"]
        );
        assert_eq!(response.header("Content-Type"), Some("video/webm"));
        assert_eq!(
            response.header("Content-Range"),
            Some(format!("bytes {start}-{end}/{size}").as_str())
        );
        assert_eq!(
            response.header("Content-Length"),
            Some((end - start + 1).to_string().as_str())
        );
        assert_eq!(response.body, clip[start..=end], "{range}");
    }
    let head = get(port, "HEAD", "/media/clip.webm", &["Range: bytes=10-19"]);
    assert_eq!(head.status_line, "HTTP/1.0 206 Partial Content");
    assert_eq!(head.header("Content-Length"), Some("10"));
    assert!(head.body.is_empty());
}

#[test]
fn refuses_unsatisfiable_ranges() {
    let (_dir, port) = fixture();
    for (path, range, size) in [
        ("/media/clip.webm", "bytes=200000-", 200_000),
        (
            "/media/clip.webm",
            "bytes=99999999999999999999999-",
            200_000,
        ),
        ("/media/clip.webm", "bytes=10-5", 200_000),
        ("/media/clip.webm", "bytes=-0", 200_000),
        ("/media/empty.mp4", "bytes=0-", 0),
        ("/media/empty.mp4", "bytes=-5", 0),
    ] {
        let header = format!("Range: {range}");
        let response = get(port, "GET", path, &[&header]);
        assert_eq!(
            response.status_line, "HTTP/1.0 416 Requested Range Not Satisfiable",
            "{range}"
        );
        assert_common_headers(&response);
        assert_eq!(
            response.header("Content-Range"),
            Some(format!("bytes */{size}").as_str())
        );
        assert_eq!(response.header("Content-Length"), Some("0"));
        assert!(response.body.is_empty());
    }
}

#[test]
fn ignores_other_ranges() {
    let (_dir, port) = fixture();
    for range in ["bytes=0-1,5-6", "items=0-1", "bytes=-", "bytes=a-", ""] {
        let header = format!("Range: {range}");
        let response = get(port, "GET", "/media/clip.webm", &[&header]);
        assert_eq!(response.status_line, "HTTP/1.0 200 OK", "{range:?}");
        assert_eq!(response.body.len(), 200_000);
    }
    // Only files take ranges: a directory gets its index, a missing file a 404.
    let index = get(port, "GET", "/indexed/", &["Range: bytes=0-1"]);
    assert_eq!(index.status_line, "HTTP/1.0 200 OK");
    assert_eq!(index.text(), "index page\n");
    let missing = get(port, "GET", "/media/none.webm", &["Range: bytes=0-1"]);
    assert_eq!(missing.status_line, "HTTP/1.0 404 File not found");
}

#[test]
fn private_asks_for_basic_authentication() {
    let (_dir, port) = fixture();
    for (path, authorization) in [
        ("/private", None),
        ("/private", Some("Authorization: Basic Ym9iOnNlY3JldA==")),
        ("/private", Some("Authorization: basic YWxpY2U6c2VjcmV0")),
        (
            "/private?next=1",
            Some("Authorization: Basic YWxpY2U6c2VjcmV0 "),
        ),
    ] {
        let headers: Vec<&str> = authorization.into_iter().collect();
        let response = get(port, "GET", path, &headers);
        assert_eq!(
            response.status_line, "HTTP/1.0 401 Unauthorized",
            "{authorization:?}"
        );
        assert_common_headers(&response);
        assert_eq!(
            response.header_names()[2..5],
            ["WWW-Authenticate", "Content-Type", "Content-Length"]
        );
        assert_eq!(
            response.header("WWW-Authenticate"),
            Some("Basic realm=\"Mercury test realm\"")
        );
        assert_eq!(
            response.header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(response.text(), "Sign-in required.\n");
    }
    for path in ["/private", "/private?x=1"] {
        let response = get(
            port,
            "GET",
            path,
            &["authorization:   Basic YWxpY2U6c2VjcmV0"],
        );
        assert_eq!(response.status_line, "HTTP/1.0 200 OK");
        assert_eq!(response.header("WWW-Authenticate"), None);
        assert_eq!(response.header("Content-Length"), Some("11"));
        assert_eq!(response.text(), "Signed in.\n");
    }
    let head = get(port, "HEAD", "/private", &[]);
    assert_eq!(head.status_line, "HTTP/1.0 401 Unauthorized");
    assert_eq!(head.header("Content-Length"), Some("18"));
    assert!(head.body.is_empty());
    // Only that exact path: the route is not a directory.
    assert_eq!(
        get(port, "GET", "/private/", &[]).status_line,
        "HTTP/1.0 404 File not found"
    );
    assert_eq!(
        get(port, "GET", "/private#x", &[]).status_line,
        "HTTP/1.0 404 File not found"
    );
    // Other methods are not implemented there either.
    assert_eq!(
        get(port, "POST", "/private", &[]).status_line,
        "HTTP/1.0 501 Unsupported method ('POST')"
    );
}

#[test]
fn redirects_directories_and_lists_them() {
    let (_dir, port) = fixture();
    for (path, location) in [
        ("/pages", "/pages/"),
        ("/pages?a=b#c", "/pages/?a=b#c"),
        ("/pages?", "/pages/"),
        ("/indexed", "/indexed/"),
    ] {
        let response = get(port, "GET", path, &[]);
        assert_eq!(
            response.status_line, "HTTP/1.0 301 Moved Permanently",
            "{path}"
        );
        assert_eq!(response.header("Location"), Some(location));
        assert_eq!(response.header("Content-Length"), Some("0"));
        assert!(response.body.is_empty());
    }
    let index = get(port, "GET", "/indexed/", &[]);
    assert_eq!(index.header("Content-type"), Some("text/html"));
    assert_eq!(index.text(), "index page\n");

    let listing = get(port, "GET", "/pages/?q=1", &[]);
    assert_eq!(listing.status_line, "HTTP/1.0 200 OK");
    assert_eq!(
        listing.header("Content-type"),
        Some("text/html; charset=utf-8")
    );
    let text = listing.text();
    assert_eq!(
        listing.header("Content-Length"),
        Some(text.len().to_string().as_str())
    );
    assert!(
        text.contains("<title>Directory listing for /pages/?q=1</title>"),
        "{text}"
    );
    let entries: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("<li>"))
        .collect();
    assert_eq!(
        entries,
        [
            "<li><a href=\"b%26%3C.txt\">b&amp;&lt;.txt</a></li>",
            "<li><a href=\"hello%20world.txt\">hello world.txt</a></li>",
            "<li><a href=\"nested%20dir/\">nested dir/</a></li>",
            "<li><a href=\"page.html\">page.html</a></li>",
            "<li><a href=\"Zed.js\">Zed.js</a></li>",
        ]
    );
    let root = get(port, "GET", "/", &[]);
    assert!(
        root.text()
            .contains("<li><a href=\"media/\">media/</a></li>")
    );
}

#[test]
fn not_found_pages_and_traversal() {
    let (_dir, port) = fixture();
    let response = get(port, "GET", "/pages/missing.html", &[]);
    assert_eq!(response.status_line, "HTTP/1.0 404 File not found");
    assert_common_headers(&response);
    assert_eq!(
        response.header_names()[2..5],
        ["Connection", "Content-Type", "Content-Length"]
    );
    assert_eq!(response.header("Connection"), Some("close"));
    assert_eq!(
        response.header("Content-Type"),
        Some("text/html;charset=utf-8")
    );
    let text = response.text();
    assert!(text.contains("<title>Error 404</title>"));
    assert!(text.contains("<p>File not found</p>"));
    assert!(
        get(port, "HEAD", "/pages/missing.html", &[])
            .body
            .is_empty()
    );
    // A file named with a trailing slash is not found.
    assert_eq!(
        get(port, "GET", "/pages/page.html/", &[]).status_line,
        "HTTP/1.0 404 File not found"
    );
    for path in [
        "/../secret.txt",
        "/pages/../../secret.txt",
        "/%2e%2e/secret.txt",
        "/..%2Fsecret.txt",
    ] {
        let response = get(port, "GET", path, &[]);
        assert_eq!(
            response.status_line, "HTTP/1.0 404 File not found",
            "{path}"
        );
    }
    assert_eq!(
        get(port, "GET", "/media/../pages/page.html", &[]).text(),
        PAGE
    );
}

#[test]
fn if_modified_since() {
    let (_dir, port) = fixture();
    let future = format!(
        "If-Modified-Since: {}",
        httpdate::fmt_http_date(std::time::UNIX_EPOCH + Duration::from_secs(4_102_444_800))
    );
    let not_modified = get(port, "GET", "/pages/page.html", &[&future]);
    assert_eq!(not_modified.status_line, "HTTP/1.0 304 Not Modified");
    assert_eq!(
        not_modified.header_names(),
        ["Server", "Date", "Accept-Ranges", "Cache-Control"]
    );
    assert!(not_modified.body.is_empty());
    for headers in [
        vec!["If-Modified-Since: Thu, 01 Jan 1970 00:00:00 GMT"],
        vec![future.as_str(), "If-None-Match: \"x\""],
        vec!["If-Modified-Since: soon"],
        vec!["If-Modified-Since: Fri, 01 Jan 2100 00:00:00 +0100"],
    ] {
        let response = get(port, "GET", "/pages/page.html", &headers);
        assert_eq!(response.status_line, "HTTP/1.0 200 OK", "{headers:?}");
        assert_eq!(response.text(), PAGE);
    }
}

#[test]
fn malformed_requests() {
    let (_dir, port) = fixture();
    let status = |request: &[u8]| {
        let raw = exchange(port, request);
        String::from_utf8_lossy(&raw)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(
        status(b"DELETE /pages/page.html HTTP/1.1\r\n\r\n"),
        "HTTP/1.0 501 Unsupported method ('DELETE')"
    );
    assert_eq!(
        status(b"get /pages/page.html HTTP/1.1\r\n\r\n"),
        "HTTP/1.0 501 Unsupported method ('get')"
    );
    assert_eq!(
        status(b"GET /a b HTTP/1.1\r\n\r\n"),
        "HTTP/1.0 400 Bad request syntax ('GET /a b HTTP/1.1')"
    );
    assert!(exchange(port, b"").is_empty());
    assert!(exchange(port, b"\r\n").is_empty());

    // Errors found before a valid version are sent as HTTP/0.9: the page only.
    let bad_version = String::from_utf8(exchange(port, b"GET / HTTP/x\r\n\r\n")).unwrap();
    assert!(bad_version.starts_with("<!DOCTYPE html>"), "{bad_version}");
    assert!(bad_version.contains("<p>Bad request version ('HTTP/x')</p>"));
    let too_new = String::from_utf8(exchange(port, b"GET / HTTP/2.0\r\n\r\n")).unwrap();
    assert!(too_new.contains("<title>Error 505</title>"));
    assert!(too_new.contains("<p>Invalid HTTP version (2.0)</p>"));
    let bad_09 = String::from_utf8(exchange(port, b"POST /x\r\n\r\n")).unwrap();
    assert!(bad_09.contains("<p>Bad HTTP/0.9 request type ('POST')</p>"));
    // An HTTP/0.9 request gets the file without a status line or headers.
    assert_eq!(
        String::from_utf8(exchange(port, b"GET /pages/page.html\r\n\r\n")).unwrap(),
        PAGE
    );

    let mut long_line = b"GET /".to_vec();
    long_line.resize(65_537, b'a');
    assert_eq!(status(&long_line), "HTTP/1.0 414 Request-URI Too Long");

    let mut long_header = b"GET / HTTP/1.1\r\nX: ".to_vec();
    let line_start = long_header.len() - 3;
    long_header.resize(line_start + 65_537, b'a');
    let response = parse(exchange(port, &long_header));
    assert_eq!(response.status_line, "HTTP/1.0 431 Line too long");
    assert!(
        response
            .text()
            .contains("got more than 65536 bytes when reading header line")
    );

    let mut many = b"GET / HTTP/1.1\r\n".to_vec();
    for i in 0..100 {
        many.extend_from_slice(format!("X-{i}: y\r\n").as_bytes());
    }
    many.extend_from_slice(b"\r\n");
    let response = parse(exchange(port, &many));
    assert_eq!(response.status_line, "HTTP/1.0 431 Too many headers");
    assert!(response.text().contains("got more than 100 headers"));
    let mut enough = b"GET /pages/page.html HTTP/1.1\r\n".to_vec();
    for i in 0..99 {
        enough.extend_from_slice(format!("X-{i}: y\r\n").as_bytes());
    }
    enough.extend_from_slice(b"\r\n");
    assert_eq!(parse(exchange(port, &enough)).text(), PAGE);
}

#[test]
fn one_request_per_connection() {
    let (_dir, port) = fixture();
    let raw = exchange(
        port,
        b"GET /pages/page.html HTTP/1.1\r\nConnection: keep-alive\r\n\r\nGET /pages/page.html HTTP/1.1\r\n\r\n",
    );
    let response = parse(raw);
    assert_eq!(response.status_line, "HTTP/1.0 200 OK");
    assert_eq!(response.text(), PAGE);
    assert_eq!(
        response.raw.windows(8).filter(|w| w == b"HTTP/1.0").count(),
        1
    );
}

#[test]
fn a_slow_client_does_not_hold_up_the_others() {
    let (_dir, port) = fixture();
    let mut slow = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    slow.write_all(b"GET /pages/page.html HTTP/1.1\r\n")
        .expect("send");
    let other = get(port, "GET", "/media/tone.wav", &[]);
    assert_eq!(other.body, b"RIFF0123WAVE");
    slow.write_all(b"\r\n").expect("send");
    let mut raw = Vec::new();
    slow.read_to_end(&mut raw).expect("read");
    assert_eq!(parse(raw).text(), PAGE);
}

#[test]
fn serves_the_repository_tests_directory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests");
    let port = Server::bind(&root, 0).expect("bind").spawn();
    let page = get(port, "GET", "/pages/http-auth.html", &[]);
    assert_eq!(page.status_line, "HTTP/1.0 200 OK");
    assert!(page.text().contains("/private"));
    let tone = fs::read(root.join("media/tone.wav")).unwrap();
    let range = get(port, "GET", "/media/tone.wav", &["Range: bytes=0-3"]);
    assert_eq!(range.body, tone[..4]);
    assert_eq!(range.body, b"RIFF");
}
