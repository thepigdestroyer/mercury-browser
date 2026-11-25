// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `/net/*` routes the network-policy work (lane net-1) added, so that
//! `tests/pages/net-cookies.html` and `tests/pages/net-request-path.html` can see what a request carried and
//! set cookies with chosen attributes:
//!
//! - `GET /net/echo` answers a JSON object with the request headers `Cookie`, `Sec-GPC`, `Referer`,
//!   `Origin`, `Authorization` and `Host` (lower-case names; a header the request did not carry is left
//!   out);
//! - `GET /net/set-cookie?name=N&value=V&attrs=A` answers with `Set-Cookie: N=V; A` (`attrs` is the raw
//!   attribute text, such as `SameSite=None; Secure; Path=/`; `value` and `attrs` may be left out). Anything
//!   that is not plain printable ASCII in a parameter is refused with 400: the values go into a header;
//! - `GET /net/auth` asks for HTTP Basic authentication (realm `net-1`) and accepts `alice` / `secret`
//!   (the credentials of `/private`): 401 with a challenge, or 200;
//! - `GET /net/redirect?to=URL` answers 302 to an `http` or `https` URL, so that a page can see how a
//!   redirect across sites is judged (an addition the cookie page asks for).
//!
//! Every `/net/*` answer carries `Access-Control-Allow-Origin` (the request's own `Origin`, never `*`, since
//! the page asks with credentials) and `Access-Control-Allow-Credentials: true`, so a page on another host
//! (`a.test` reading `b.test`) can read it. Only a **local** origin gets them (`localhost`, `127.0.0.1`,
//! `[::1]`, and the `.test` and `.localhost` names): `/net/echo` shows the cookies the browser holds for
//! this host, which a page of the open web must not be able to read while the server runs. `OPTIONS` on a
//! `/net/*` path answers the preflight the same way. Every request to a `/net/*` path is written to the
//! request log.

use std::io;

use serde_json::{Map, Value};

use super::PRIVATE_CREDENTIALS;
use super::downloads::{RequestLog, parse_query, record, url_path};
use super::files::split_url;
use super::http::{Request, Responder, reason};

/// The path prefix of these routes.
pub(super) const NET_PREFIX: &str = "/net/";
/// The realm of `/net/auth`.
pub(super) const NET_AUTH_REALM: &str = "net-1";

/// The request headers `/net/echo` reports: the key in the answer, and the header it is read from.
const ECHOED: [(&str, &str); 6] = [
    ("cookie", "Cookie"),
    ("sec-gpc", "Sec-GPC"),
    ("referer", "Referer"),
    ("origin", "Origin"),
    ("authorization", "Authorization"),
    ("host", "Host"),
];

/// Whether a request target is one of these routes.
pub(super) fn is_net_path(target: &str) -> bool {
    url_path(target).starts_with(NET_PREFIX)
}

/// Whether text can go into a header as it is: printable ASCII and nothing else (no control character,
/// so no line break, and no byte the server would have to guess the encoding of).
fn is_plain(text: &str) -> bool {
    text.chars().all(|c| (' '..='~').contains(&c))
}

/// Whether an `Origin` header names a page that may read these routes: `http` or `https` on `localhost`,
/// `127.0.0.1`, `[::1]` or a `.test` or `.localhost` name, with any port.
pub(super) fn is_local_origin(origin: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    // An origin is a scheme, a host and a port: no user name, path, query or fragment.
    if (scheme != "http" && scheme != "https") || rest.contains(['@', '/', '?', '#', '\\']) {
        return false;
    }
    let host = if rest.starts_with('[') {
        &rest[..rest.find(']').map_or(rest.len(), |end| end + 1)]
    } else {
        rest.split(':').next().unwrap_or_default()
    };
    let host = host.to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
        || host.ends_with(".test")
        || host.ends_with(".localhost")
}

/// The CORS headers of an answer: the request's own local origin, with credentials; nothing for any other.
fn cors_headers(request: &Request) -> Vec<(&'static str, String)> {
    match request.header("Origin") {
        Some(origin) if is_local_origin(origin) && is_plain(origin) => vec![
            ("Access-Control-Allow-Origin", origin.to_owned()),
            ("Access-Control-Allow-Credentials", "true".into()),
            ("Vary", "Origin".into()),
        ],
        _ => Vec::new(),
    }
}

/// An answer with a text body, the CORS headers, and `extra` headers.
fn send_text(
    responder: &mut Responder<'_>,
    request: &Request,
    code: u16,
    phrase: &str,
    content_type: &str,
    body: &str,
    extra: Vec<(&'static str, String)>,
) -> io::Result<()> {
    let mut headers = extra;
    headers.push(("Content-Type", content_type.to_owned()));
    headers.push(("Content-Length", body.len().to_string()));
    headers.extend(cors_headers(request));
    responder.send_head(code, phrase, &headers)?;
    responder.send_body(body.as_bytes())
}

/// A refusal with the CORS headers, so that a page that is allowed to read the routes can read the refusal.
fn send_refusal(responder: &mut Responder<'_>, request: &Request, message: &str) -> io::Result<()> {
    send_text(
        responder,
        request,
        400,
        reason(400),
        "text/plain; charset=utf-8",
        &format!("{message}\n"),
        Vec::new(),
    )
}

/// Routes a `GET` or `HEAD` of a `/net/*` path, and writes it to the request log.
pub(super) fn respond(
    responder: &mut Responder<'_>,
    request: &Request,
    log: Option<&RequestLog>,
) -> io::Result<()> {
    let result = match &url_path(&request.path)[NET_PREFIX.len()..] {
        "echo" => send_echo(responder, request),
        "set-cookie" => send_set_cookie(responder, request),
        "auth" => send_auth(responder, request),
        "redirect" => send_redirect(responder, request),
        _ => responder.send_error(404, None, None),
    };
    record(log, request, None, None);
    result
}

/// `OPTIONS` of a `/net/*` path: the answer to a CORS preflight.
pub(super) fn preflight(
    responder: &mut Responder<'_>,
    request: &Request,
    log: Option<&RequestLog>,
) -> io::Result<()> {
    let mut extra = vec![
        (
            "Access-Control-Allow-Methods",
            "GET, HEAD, OPTIONS".to_owned(),
        ),
        ("Access-Control-Max-Age", "0".to_owned()),
    ];
    if let Some(requested) = request
        .header("Access-Control-Request-Headers")
        .filter(|requested| is_plain(requested))
    {
        extra.push(("Access-Control-Allow-Headers", requested.to_owned()));
    }
    let result = send_text(
        responder,
        request,
        200,
        reason(200),
        "text/plain; charset=utf-8",
        "",
        extra,
    );
    record(log, request, None, None);
    result
}

/// `/net/echo`: the headers the request carried, as JSON.
fn send_echo(responder: &mut Responder<'_>, request: &Request) -> io::Result<()> {
    let mut seen = Map::new();
    for (key, name) in ECHOED {
        if let Some(value) = request.header(name) {
            seen.insert(key.to_owned(), Value::from(value));
        }
    }
    let body = format!("{}\n", Value::Object(seen));
    send_text(
        responder,
        request,
        200,
        reason(200),
        "application/json",
        &body,
        Vec::new(),
    )
}

/// `/net/set-cookie?name=N&value=V&attrs=A`.
fn send_set_cookie(responder: &mut Responder<'_>, request: &Request) -> io::Result<()> {
    let query = parse_query(split_url(&request.path).1);
    let Some(name) = query.get("name") else {
        return send_refusal(responder, request, "name is required");
    };
    let value = query.get("value").map_or("", String::as_str);
    let attrs = query.get("attrs").map_or("", String::as_str);
    if !(is_plain(name) && is_plain(value) && is_plain(attrs)) {
        return send_refusal(
            responder,
            request,
            "name, value and attrs must be printable ASCII",
        );
    }
    let cookie = if attrs.is_empty() {
        format!("{name}={value}")
    } else {
        format!("{name}={value}; {attrs}")
    };
    let body = format!("{}\n", serde_json::json!({ "set": cookie }));
    send_text(
        responder,
        request,
        200,
        reason(200),
        "application/json",
        &body,
        vec![("Set-Cookie", cookie)],
    )
}

/// `/net/auth`: 200 for `alice` / `secret`, otherwise 401 and a challenge.
fn send_auth(responder: &mut Responder<'_>, request: &Request) -> io::Result<()> {
    let signed_in = request.header("Authorization") == Some(PRIVATE_CREDENTIALS);
    if signed_in {
        return send_text(
            responder,
            request,
            200,
            reason(200),
            "text/plain; charset=utf-8",
            "Signed in.\n",
            Vec::new(),
        );
    }
    send_text(
        responder,
        request,
        401,
        reason(401),
        "text/plain; charset=utf-8",
        "Sign-in required.\n",
        vec![(
            "WWW-Authenticate",
            format!("Basic realm=\"{NET_AUTH_REALM}\""),
        )],
    )
}

/// `/net/redirect?to=URL`: 302 to an `http` or `https` URL.
fn send_redirect(responder: &mut Responder<'_>, request: &Request) -> io::Result<()> {
    let query = parse_query(split_url(&request.path).1);
    let Some(target) = query.get("to").filter(|target| {
        (target.starts_with("http://") || target.starts_with("https://")) && is_plain(target)
    }) else {
        return send_refusal(responder, request, "to must be an http or https URL");
    };
    send_text(
        responder,
        request,
        302,
        "Found",
        "text/plain; charset=utf-8",
        "",
        vec![("Location", target.clone())],
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpStream};
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::super::{PRIVATE_REALM, Server};
    use super::*;

    struct Answer {
        status: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Answer {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }

        fn json(&self) -> Value {
            serde_json::from_str(&self.body).expect("a JSON body")
        }
    }

    fn server() -> (tempfile::TempDir, u16) {
        let dir = tempfile::tempdir().expect("a directory");
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/page.html"), "<p>page</p>\n").unwrap();
        let port = Server::bind(dir.path(), 0).expect("bind").spawn();
        (dir, port)
    }

    fn ask(port: u16, method: &str, path: &str, headers: &[&str]) -> Answer {
        let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
        for header in headers {
            request.push_str(header);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        stream.write_all(request.as_bytes()).expect("send");
        stream.shutdown(Shutdown::Write).expect("shutdown");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("an answer");
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").expect("a header block");
        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap_or_default().to_owned();
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(": ").expect("a header");
                (name.to_owned(), value.to_owned())
            })
            .collect();
        Answer {
            status,
            headers,
            body: body.to_owned(),
        }
    }

    fn get(port: u16, path: &str, headers: &[&str]) -> Answer {
        ask(port, "GET", path, headers)
    }

    #[test]
    fn the_echo_reports_the_six_headers_it_was_given() {
        let (_dir, port) = server();
        let answer = get(
            port,
            "/net/echo",
            &[
                "Cookie: a=1; b=2",
                "Sec-GPC: 1",
                "Referer: http://a.test:8765/pages/x.html",
                "Origin: http://a.test:8765",
                "Authorization: Basic YWxpY2U6c2VjcmV0",
                "User-Agent: not echoed",
            ],
        );

        assert_eq!(answer.status, "HTTP/1.0 200 OK");
        assert_eq!(answer.header("Content-Type"), Some("application/json"));
        let json = answer.json();
        assert_eq!(json["cookie"], "a=1; b=2");
        assert_eq!(json["sec-gpc"], "1");
        assert_eq!(json["referer"], "http://a.test:8765/pages/x.html");
        assert_eq!(json["origin"], "http://a.test:8765");
        assert_eq!(json["authorization"], "Basic YWxpY2U6c2VjcmV0");
        assert_eq!(json["host"], format!("127.0.0.1:{port}"));
        assert_eq!(json.as_object().unwrap().len(), 6, "{json}");
        assert_eq!(
            answer.header("Content-Length"),
            Some(answer.body.len().to_string().as_str())
        );
    }

    #[test]
    fn a_header_the_request_did_not_carry_is_not_in_the_echo() {
        let (_dir, port) = server();
        let json = get(port, "/net/echo?keep=1", &[]).json();

        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec!["host"],
            "only the Host header the client always sends"
        );
        // Not null: a page tells "absent" from "empty" with `=== undefined`.
        assert!(json.get("sec-gpc").is_none());
        assert!(json.get("cookie").is_none());
    }

    #[test]
    fn head_answers_the_headers_and_no_body() {
        let (_dir, port) = server();
        let answer = ask(port, "HEAD", "/net/echo", &["Cookie: a=1"]);
        assert_eq!(answer.status, "HTTP/1.0 200 OK");
        assert!(answer.header("Content-Length").is_some());
        assert_eq!(answer.body, "");
    }

    #[test]
    fn only_a_local_origin_may_read_the_routes() {
        let (_dir, port) = server();
        for origin in [
            "http://a.test:8765",
            "http://b.test",
            "https://deep.sub.test:1",
            "http://localhost:8765",
            "http://127.0.0.1:3000",
            "http://[::1]:8765",
            "http://app.localhost",
        ] {
            let answer = get(port, "/net/echo", &[&format!("Origin: {origin}")]);
            assert_eq!(
                answer.header("Access-Control-Allow-Origin"),
                Some(origin),
                "{origin}"
            );
            assert_eq!(
                answer.header("Access-Control-Allow-Credentials"),
                Some("true"),
                "{origin}"
            );
            assert_eq!(answer.header("Vary"), Some("Origin"), "{origin}");
        }
        for origin in [
            "https://evil.example",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example",
            "http://notlocalhost",
            "null",
            "file://",
            "ftp://localhost",
            "http://a.test.evil.example",
        ] {
            let answer = get(port, "/net/echo", &[&format!("Origin: {origin}")]);
            assert_eq!(
                answer.header("Access-Control-Allow-Origin"),
                None,
                "{origin}"
            );
            assert_eq!(
                answer.header("Access-Control-Allow-Credentials"),
                None,
                "{origin}"
            );
        }
        // No Origin, no CORS headers; never a wildcard, which credentials do not allow.
        let answer = get(port, "/net/echo", &[]);
        assert_eq!(answer.header("Access-Control-Allow-Origin"), None);
    }

    #[test]
    fn the_cors_headers_are_on_every_net_answer() {
        let (_dir, port) = server();
        let origin = "Origin: http://a.test:8765";
        for path in [
            "/net/echo",
            "/net/set-cookie?name=a&value=1",
            "/net/auth",
            "/net/redirect?to=http%3A%2F%2Fb.test%3A8765%2Fnet%2Fecho",
            "/net/set-cookie",
            "/net/redirect",
        ] {
            let answer = get(port, path, &[origin]);
            assert_eq!(
                answer.header("Access-Control-Allow-Origin"),
                Some("http://a.test:8765"),
                "{path}: {}",
                answer.status
            );
        }
    }

    #[test]
    fn the_preflight_is_answered() {
        let (_dir, port) = server();
        let answer = ask(
            port,
            "OPTIONS",
            "/net/echo",
            &[
                "Origin: http://a.test:8765",
                "Access-Control-Request-Method: GET",
                "Access-Control-Request-Headers: authorization, x-probe",
            ],
        );
        assert_eq!(answer.status, "HTTP/1.0 200 OK");
        assert_eq!(
            answer.header("Access-Control-Allow-Origin"),
            Some("http://a.test:8765")
        );
        assert_eq!(
            answer.header("Access-Control-Allow-Methods"),
            Some("GET, HEAD, OPTIONS")
        );
        assert_eq!(
            answer.header("Access-Control-Allow-Headers"),
            Some("authorization, x-probe")
        );
        assert_eq!(answer.body, "");

        // From an origin that may not read the routes there is no permission.
        let refused = ask(
            port,
            "OPTIONS",
            "/net/echo",
            &["Origin: https://evil.example"],
        );
        assert_eq!(refused.header("Access-Control-Allow-Origin"), None);
    }

    #[test]
    fn options_elsewhere_is_still_not_implemented() {
        let (_dir, port) = server();
        let answer = ask(port, "OPTIONS", "/pages/page.html", &[]);
        assert!(
            answer.status.starts_with("HTTP/1.0 501"),
            "{}",
            answer.status
        );
        let answer = ask(port, "OPTIONS", "/private", &[]);
        assert!(
            answer.status.starts_with("HTTP/1.0 501"),
            "{}",
            answer.status
        );
    }

    #[test]
    fn set_cookie_sets_the_cookie_with_the_attributes_as_given() {
        let (_dir, port) = server();

        let plain = get(port, "/net/set-cookie?name=fp&value=1", &[]);
        assert_eq!(plain.status, "HTTP/1.0 200 OK");
        assert_eq!(plain.header("Set-Cookie"), Some("fp=1"));
        assert_eq!(plain.json()["set"], "fp=1");

        let with_attributes = get(
            port,
            "/net/set-cookie?name=tp&value=x&attrs=SameSite%3DNone%3B%20Secure%3B%20Path%3D%2F",
            &[],
        );
        assert_eq!(
            with_attributes.header("Set-Cookie"),
            Some("tp=x; SameSite=None; Secure; Path=/")
        );

        // The attributes `+` stands for a space, as in any query; the value may be left out.
        let empty = get(port, "/net/set-cookie?name=e&attrs=Max-Age%3D0", &[]);
        assert_eq!(empty.header("Set-Cookie"), Some("e=; Max-Age=0"));
        let blank = get(port, "/net/set-cookie?name=b&value=&attrs=", &[]);
        assert_eq!(blank.header("Set-Cookie"), Some("b="));
        let plus = get(port, "/net/set-cookie?name=p&value=a+b", &[]);
        assert_eq!(plus.header("Set-Cookie"), Some("p=a b"));
    }

    #[test]
    fn set_cookie_refuses_what_cannot_go_into_a_header() {
        let (_dir, port) = server();
        for path in [
            // No name.
            "/net/set-cookie",
            "/net/set-cookie?value=1",
            // A line break, a control character, a DEL, a non-ASCII character: in each parameter.
            "/net/set-cookie?name=a&value=%0d%0aSet-Cookie%3A%20x%3D1",
            "/net/set-cookie?name=a%0d%0ab&value=1",
            "/net/set-cookie?name=a&value=1&attrs=Path%3D%2F%0d%0aX%3A%20y",
            "/net/set-cookie?name=a&value=%09",
            "/net/set-cookie?name=a&value=%7f",
            "/net/set-cookie?name=a&value=%00",
            "/net/set-cookie?name=a&value=caf%C3%A9",
            "/net/set-cookie?name=a&value=1&attrs=Path%3D%C3%A9",
        ] {
            let answer = get(port, path, &[]);
            assert!(
                answer.status.starts_with("HTTP/1.0 400"),
                "{path}: {}",
                answer.status
            );
            assert_eq!(answer.header("Set-Cookie"), None, "{path}");
        }
    }

    #[test]
    fn auth_asks_for_basic_credentials_and_accepts_alice() {
        let (_dir, port) = server();

        let challenge = get(port, "/net/auth", &[]);
        assert_eq!(challenge.status, "HTTP/1.0 401 Unauthorized");
        assert_eq!(
            challenge.header("WWW-Authenticate"),
            Some("Basic realm=\"net-1\"")
        );

        let wrong = get(
            port,
            "/net/auth",
            &["Authorization: Basic YWxpY2U6d3Jvbmc="],
        );
        assert_eq!(wrong.status, "HTTP/1.0 401 Unauthorized");

        // base64("alice:secret")
        let right = get(
            port,
            "/net/auth",
            &["Authorization: Basic YWxpY2U6c2VjcmV0"],
        );
        assert_eq!(right.status, "HTTP/1.0 200 OK");
        assert_eq!(right.header("WWW-Authenticate"), None);
        assert_eq!(right.body, "Signed in.\n");

        // `/private` keeps its own realm.
        let private = get(port, "/private", &[]);
        assert_eq!(
            private.header("WWW-Authenticate"),
            Some("Basic realm=\"Mercury test realm\"")
        );
        assert_ne!(NET_AUTH_REALM, PRIVATE_REALM);
    }

    #[test]
    fn redirect_goes_to_an_http_or_https_url_and_nothing_else() {
        let (_dir, port) = server();

        let redirect = get(
            port,
            "/net/redirect?to=http%3A%2F%2Fb.test%3A8765%2Fnet%2Fecho%3Fx%3D1",
            &[],
        );
        assert_eq!(redirect.status, "HTTP/1.0 302 Found");
        assert_eq!(
            redirect.header("Location"),
            Some("http://b.test:8765/net/echo?x=1")
        );
        let secure = get(port, "/net/redirect?to=https%3A%2F%2Fb.test%2F", &[]);
        assert_eq!(secure.header("Location"), Some("https://b.test/"));

        for path in [
            "/net/redirect",
            "/net/redirect?to=",
            "/net/redirect?to=javascript%3Aalert(1)",
            "/net/redirect?to=file%3A%2F%2F%2Fetc%2Fpasswd",
            "/net/redirect?to=%2Fnet%2Fecho",
            "/net/redirect?to=http%3A%2F%2Fb.test%2F%0d%0aX%3A%20y",
        ] {
            let answer = get(port, path, &[]);
            assert!(
                answer.status.starts_with("HTTP/1.0 400"),
                "{path}: {}",
                answer.status
            );
            assert_eq!(answer.header("Location"), None, "{path}");
        }
    }

    #[test]
    fn an_unknown_net_route_is_not_found_and_a_post_is_refused_as_before() {
        let (_dir, port) = server();
        assert!(
            get(port, "/net/nothing", &[])
                .status
                .starts_with("HTTP/1.0 404")
        );
        assert!(get(port, "/net/", &[]).status.starts_with("HTTP/1.0 404"));
        // `/net` without the slash is a path like any other (a file or a 404), and `/network` is not a route.
        assert!(
            get(port, "/network/echo", &[])
                .status
                .starts_with("HTTP/1.0 404")
        );

        let post = format!(
            "POST /net/echo HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n"
        );
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(post.as_bytes()).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.0 405"), "{raw}");
    }

    #[test]
    fn the_files_of_the_test_directory_are_served_as_before() {
        let (_dir, port) = server();
        let page = get(port, "/pages/page.html", &["Origin: http://a.test:8765"]);
        assert_eq!(page.status, "HTTP/1.0 200 OK");
        assert_eq!(page.body, "<p>page</p>\n");
        // CORS is for the /net/* routes only.
        assert_eq!(page.header("Access-Control-Allow-Origin"), None);
    }

    /// The line of the log for a request of this method and path.
    fn line_of<'a>(lines: &'a [Value], method: &str, path: &str) -> &'a Value {
        lines
            .iter()
            .find(|line| line["method"] == method && line["path"] == path)
            .unwrap_or_else(|| panic!("no line for {method} {path}: {lines:#?}"))
    }

    /// The lines of the request log, as JSON, waiting up to ten seconds for `count` of them.
    fn log_lines(path: &Path, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let lines: Vec<Value> = fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .map(|line| serde_json::from_str(line).expect("a JSON line"))
                .collect();
            if lines.len() >= count || Instant::now() > deadline {
                return lines;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_request_log_has_sec_gpc_origin_and_host_and_logs_the_net_routes() {
        let dir = tempfile::tempdir().expect("a directory");
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/page.html"), "<p>page</p>\n").unwrap();
        let log = dir.path().join("requests.jsonl");
        let port = Server::bind(dir.path(), 0)
            .expect("bind")
            .with_request_log(&log)
            .spawn();

        get(
            port,
            "/net/echo?keep=1",
            &[
                "Sec-GPC: 1",
                "Origin: http://a.test:8765",
                "Cookie: sid=1",
                "Referer: http://a.test:8765/",
            ],
        );
        get(port, "/net/set-cookie?name=a&value=1", &[]);
        get(port, "/net/auth", &[]);
        get(port, "/net/nothing", &[]);
        ask(
            port,
            "OPTIONS",
            "/net/echo",
            &["Origin: http://a.test:8765"],
        );
        get(port, "/pages/page.html", &[]);

        let lines = log_lines(&log, 6);
        assert_eq!(lines.len(), 6, "{lines:#?}");

        // Each connection writes its own line, so the lines are found by what they say, not by position.
        let host = format!("127.0.0.1:{port}");

        let echo = line_of(&lines, "GET", "/net/echo?keep=1");
        assert_eq!(echo["sec_gpc"], "1");
        assert_eq!(echo["origin"], "http://a.test:8765");
        assert_eq!(echo["host"], host);
        assert_eq!(echo["cookie"], "sid=1");
        assert_eq!(echo["referer"], "http://a.test:8765/");

        // A request without them has them as null; every route logs the host.
        let set_cookie = line_of(&lines, "GET", "/net/set-cookie?name=a&value=1");
        assert!(set_cookie["sec_gpc"].is_null());
        assert!(set_cookie["origin"].is_null());
        assert_eq!(set_cookie["host"], host);
        assert_eq!(line_of(&lines, "GET", "/net/auth")["host"], host);
        assert_eq!(line_of(&lines, "GET", "/net/nothing")["host"], host);
        let preflight = line_of(&lines, "OPTIONS", "/net/echo");
        assert_eq!(preflight["origin"], "http://a.test:8765");

        let page = line_of(&lines, "GET", "/pages/page.html");
        assert!(page["sec_gpc"].is_null());
        assert_eq!(page["host"], host);
        // The keys the log always had are still there.
        for key in [
            "method",
            "path",
            "cookie",
            "referer",
            "user_agent",
            "range",
            "sent",
            "complete",
        ] {
            assert!(page.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn is_local_origin_reads_the_host_and_nothing_else() {
        for origin in [
            "http://localhost",
            "https://LOCALHOST:8443",
            "http://127.0.0.1",
            "http://[::1]",
            "http://[::1]:80",
            "http://a.test",
            "http://a.b.test:1234",
            "http://x.localhost:1",
        ] {
            assert!(is_local_origin(origin), "{origin}");
        }
        for origin in [
            "",
            "localhost",
            "//localhost",
            "http://",
            "http://evil.example",
            "http://localhost.example",
            "http://127.0.0.1.example",
            "http://test.example",
            "http://localhost@evil.example",
            "http://localhost:80@evil.example",
            "http://localhost/path",
            "http://localhost?x=1",
            "http://evil.example#localhost",
            "http://[::2]",
            "null",
        ] {
            assert!(!is_local_origin(origin), "{origin}");
        }
    }

    #[test]
    fn is_plain_is_printable_ascii_only() {
        assert!(is_plain(""));
        assert!(is_plain("name=value; Path=/; SameSite=None; Secure"));
        for text in [
            "a\nb",
            "a\rb",
            "a\tb",
            "a\u{7f}",
            "caf\u{e9}",
            "\u{0}",
            "\u{2028}",
        ] {
            assert!(!is_plain(text), "{text:?}");
        }
    }
}
