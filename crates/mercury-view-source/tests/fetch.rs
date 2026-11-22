// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `fetch_source` against servers on the loopback interface: what it sends, how it follows
//! redirects, where it stops reading, how it reports failures, and how it treats a proxy. Nothing
//! here reaches beyond `127.0.0.1` (the proxy tests name hosts that are only ever sent to the
//! proxy, never resolved).

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use mercury_view_source::{
    FetchError, HopPolicy, HopVerdict, MAX_REDIRECTS, MAX_SHOWN_BYTES, SourceBody, SourceDocument,
    SourceKind, ViewSourceUrl, fetch_source,
};
use support::{
    Reply, Server, chunked, closed_port, html, ok, options, options_with_cookies, redirect,
    response,
};
use url::Url;

const DOCUMENT_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

fn text_of(document: &SourceDocument) -> &str {
    match &document.body {
        SourceBody::Text { text, .. } => text,
        SourceBody::Binary { bytes } => panic!("a binary body of {bytes} bytes, not text"),
    }
}

fn source(text: &str) -> ViewSourceUrl {
    ViewSourceUrl::parse(&format!("view-source:{text}")).expect("a source address")
}

// ---- the document ----

#[tokio::test]
async fn a_page_comes_back_with_its_status_type_and_text() {
    let server = Server::always(|| html("<!doctype html><title>Hi</title><p>caf\u{e9}</p>"));
    let url = server.source("/dir/page.html?q=1");

    let document = fetch_source(&url, &options()).await.expect("a document");

    assert_eq!(document.final_url, server.url("/dir/page.html?q=1"));
    assert_eq!(document.status, 200);
    assert_eq!(
        document.content_type.as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(document.kind, SourceKind::Html);
    let body = "<!doctype html><title>Hi</title><p>caf\u{e9}</p>";
    assert_eq!(
        document.body,
        SourceBody::Text {
            text: body.to_owned(),
            bytes: body.len() as u64,
            truncated: false,
            charset: "UTF-8",
        }
    );
}

#[tokio::test]
async fn error_pages_are_shown_as_source_with_their_status() {
    for (status, reason) in [(404, "Not Found"), (500, "Internal Server Error")] {
        let server = Server::always(move || {
            Reply::Bytes(response(
                &format!("{status} {reason}"),
                &[("Content-Type", "text/html")],
                b"<h1>Oops</h1>",
            ))
        });
        let document = fetch_source(&server.source("/"), &options())
            .await
            .expect("a document");
        assert_eq!(document.status, status);
        assert_eq!(text_of(&document), "<h1>Oops</h1>");
        assert_eq!(document.kind, SourceKind::Html);
    }
}

#[tokio::test]
async fn statuses_that_are_not_redirects_are_not_followed_even_with_a_location() {
    let server = Server::always(|| {
        Reply::Bytes(response(
            "300 Multiple Choices",
            &[("Location", "/elsewhere"), ("Content-Type", "text/plain")],
            b"choices",
        ))
    });
    let document = fetch_source(&server.source("/"), &options())
        .await
        .expect("a document");
    assert_eq!(document.status, 300);
    assert_eq!(text_of(&document), "choices");
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn an_empty_body_is_an_empty_text() {
    let server = Server::always(|| Reply::Bytes(response("204 No Content", &[], b"")));
    let document = fetch_source(&server.source("/"), &options())
        .await
        .expect("a document");
    assert_eq!(document.status, 204);
    assert_eq!(document.content_type, None);
    assert_eq!(
        document.body,
        SourceBody::Text {
            text: String::new(),
            bytes: 0,
            truncated: false,
            charset: "UTF-8",
        }
    );
}

#[tokio::test]
async fn a_response_without_a_type_is_sniffed_for_markup_only() {
    let markup =
        Server::always(|| Reply::Bytes(response("200 OK", &[], b"\n <!doctype html><p>x")));
    let document = fetch_source(&markup.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Html);
    assert_eq!(document.content_type, None);

    let words = Server::always(|| Reply::Bytes(response("200 OK", &[], b"just words")));
    let document = fetch_source(&words.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Plain);
}

#[tokio::test]
async fn a_chunked_body_is_joined_even_inside_a_character() {
    // "\u{f6}" (c3 b6) is split between two chunks.
    let server = Server::always(|| {
        chunked(
            "text/plain; charset=utf-8",
            &[b"Hello, ", b"chunked ", b"w\xC3", b"\xB6rld"],
        )
    });
    let document = fetch_source(&server.source("/"), &options())
        .await
        .expect("a document");
    assert_eq!(text_of(&document), "Hello, chunked w\u{f6}rld");
    assert_eq!(
        document.body,
        SourceBody::Text {
            text: "Hello, chunked w\u{f6}rld".to_owned(),
            bytes: 21,
            truncated: false,
            charset: "UTF-8",
        }
    );
}

// ---- text and binary ----

#[tokio::test]
async fn the_charset_of_the_header_decodes_the_body() {
    let latin = Server::always(|| ok("text/html; charset=windows-1252", b"<p>caf\xE9</p>"));
    let document = fetch_source(&latin.source("/"), &options()).await.unwrap();
    assert_eq!(text_of(&document), "<p>caf\u{e9}</p>");
    assert!(matches!(
        document.body,
        SourceBody::Text {
            charset: "windows-1252",
            ..
        }
    ));

    // Shift_JIS for the three characters of "Japanese" in Japanese.
    let japanese =
        Server::always(|| ok("text/plain; charset=shift_jis", b"\x93\xFA\x96\x7B\x8C\xEA"));
    let document = fetch_source(&japanese.source("/"), &options())
        .await
        .unwrap();
    assert_eq!(text_of(&document), "\u{65e5}\u{672c}\u{8a9e}");
    assert!(matches!(
        document.body,
        SourceBody::Text {
            charset: "Shift_JIS",
            ..
        }
    ));
}

#[tokio::test]
async fn a_page_in_utf16_is_text_not_binary() {
    // UTF-16LE with a byte order mark: every other byte is a NUL.
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "<p>hi</p>".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let server = Server::always(move || ok("text/html", &bytes));
    let document = fetch_source(&server.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Html);
    assert_eq!(text_of(&document), "<p>hi</p>");
    assert!(matches!(
        document.body,
        SourceBody::Text {
            charset: "UTF-16LE",
            ..
        }
    ));
}

#[tokio::test]
async fn images_and_other_binary_types_are_counted_not_shown() {
    for content_type in [
        "image/png",
        "audio/mpeg",
        "video/mp4",
        "font/woff2",
        "application/pdf",
        "application/octet-stream",
    ] {
        let server = Server::always(move || ok(content_type, &vec![b'x'; 5000]));
        let document = fetch_source(&server.source("/"), &options()).await.unwrap();
        assert_eq!(document.kind, SourceKind::Binary, "{content_type}");
        assert_eq!(document.body, SourceBody::Binary { bytes: 5000 });
        assert_eq!(document.content_type.as_deref(), Some(content_type));
        assert_eq!(document.status, 200);
    }
}

#[tokio::test]
async fn text_with_a_nul_byte_in_the_first_kilobyte_is_binary() {
    let mut inside = vec![b'a'; 1023];
    inside.push(0);
    inside.extend_from_slice(b" and more text");
    let server = Server::always(move || html_bytes(&inside));
    let document = fetch_source(&server.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Binary);
    assert_eq!(document.body, SourceBody::Binary { bytes: 1038 });

    // The 1025th byte is outside the first kilobyte: still text.
    let mut outside = vec![b'a'; 1024];
    outside.push(0);
    let server = Server::always(move || html_bytes(&outside));
    let document = fetch_source(&server.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Html);
    assert!(matches!(
        document.body,
        SourceBody::Text { bytes: 1025, .. }
    ));
}

fn html_bytes(bytes: &[u8]) -> Reply {
    ok("text/html", bytes)
}

#[tokio::test]
async fn a_binary_body_is_counted_up_to_the_cap_and_no_further() {
    let server = Server::always(|| ok("application/octet-stream", &vec![0_u8; 3 * 1024 * 1024]));
    let document = fetch_source(&server.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Binary);
    assert_eq!(
        document.body,
        SourceBody::Binary {
            bytes: MAX_SHOWN_BYTES as u64
        }
    );
}

#[tokio::test]
async fn a_body_the_server_compressed_unasked_is_not_shown_as_text() {
    let server = Server::always(|| {
        Reply::Bytes(response(
            "200 OK",
            &[("Content-Type", "text/html"), ("Content-Encoding", "gzip")],
            b"\x1F\x8B\x08\x00 plain looking bytes",
        ))
    });
    let document = fetch_source(&server.source("/"), &options()).await.unwrap();
    assert_eq!(document.kind, SourceKind::Binary);
    assert!(matches!(document.body, SourceBody::Binary { .. }));
}

// ---- the size cap ----

fn plain(len: usize) -> Reply {
    ok("text/plain", &vec![b'a'; len])
}

#[tokio::test]
async fn a_body_is_cut_at_exactly_the_cap() {
    for (len, truncated) in [
        (MAX_SHOWN_BYTES - 1, false),
        (MAX_SHOWN_BYTES, false),
        (MAX_SHOWN_BYTES + 1, true),
        (3 * 1024 * 1024, true),
    ] {
        let server = Server::always(move || plain(len));
        let document = fetch_source(&server.source("/"), &options()).await.unwrap();
        let SourceBody::Text {
            text,
            bytes,
            truncated: was_truncated,
            ..
        } = &document.body
        else {
            panic!("text expected for {len} bytes");
        };
        assert_eq!(*was_truncated, truncated, "{len} bytes");
        assert_eq!(*bytes, len.min(MAX_SHOWN_BYTES) as u64, "{len} bytes");
        assert_eq!(text.len(), len.min(MAX_SHOWN_BYTES), "{len} bytes");
    }
}

#[tokio::test]
async fn a_content_length_that_promises_more_does_not_make_the_fetch_wait() {
    // 100 MiB promised, 3 MiB sent, the connection then held open: the fetch must stop at the
    // cap and let go, not wait for the rest.
    let head = response(
        "200 OK",
        &[
            ("Content-Type", "text/plain"),
            ("Content-Length", "104857600"),
        ],
        &vec![b'a'; 3 * 1024 * 1024],
    );
    let server = Server::always(move || Reply::ThenHold(head.clone()));
    let started = Instant::now();

    let document = fetch_source(&server.source("/"), &options()).await.unwrap();

    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(matches!(
        document.body,
        SourceBody::Text {
            truncated: true,
            bytes,
            ..
        } if bytes == MAX_SHOWN_BYTES as u64
    ));
    assert!(
        server.wait_for_hangups(1, Duration::from_secs(5)).await,
        "the connection is closed once the cap is reached"
    );
}

#[tokio::test]
async fn a_content_length_that_promises_less_ends_the_body_there() {
    let server = Server::always(|| {
        Reply::Bytes(response(
            "200 OK",
            &[("Content-Type", "text/plain"), ("Content-Length", "5")],
            b"hello world, and then some",
        ))
    });
    let document = fetch_source(&server.source("/"), &options()).await.unwrap();
    assert_eq!(text_of(&document), "hello");
}

#[tokio::test]
async fn a_body_that_ends_early_is_unreachable() {
    let server = Server::always(|| {
        Reply::Bytes(response(
            "200 OK",
            &[("Content-Type", "text/plain"), ("Content-Length", "1000")],
            b"short",
        ))
    });
    assert_eq!(
        fetch_source(&server.source("/"), &options())
            .await
            .unwrap_err(),
        FetchError::Unreachable
    );
}

// ---- what is sent ----

#[tokio::test]
async fn the_request_carries_the_engines_headers_and_nothing_else() {
    let server = Server::always(|| html("<p>x</p>"));
    let (options, _) = options_with_cookies(|_| Some("sid=abc; theme=dark".to_owned()));

    fetch_source(&server.source("/p?q=1"), &options)
        .await
        .unwrap();

    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    let request = &seen[0];
    assert_eq!(request.request_line, "GET /p?q=1 HTTP/1.1");
    assert_eq!(request.header("user-agent"), Some("MercuryTest/1.0"));
    assert_eq!(request.header("accept"), Some(DOCUMENT_ACCEPT));
    assert_eq!(request.header("accept-language"), Some("en-US"));
    assert_eq!(request.header("cookie"), Some("sid=abc; theme=dark"));
    assert_eq!(
        request.header("host"),
        Some(format!("127.0.0.1:{}", server.addr.port()).as_str())
    );
    // Not a header more than these: no Referer, no Sec-Fetch-*, no Origin, no credentials, no
    // Accept-Encoding (bodies come as written), no Upgrade-Insecure-Requests.
    let mut names = request.header_names();
    names.sort_unstable();
    assert_eq!(
        names,
        ["accept", "accept-language", "cookie", "host", "user-agent"]
    );
}

#[tokio::test]
async fn without_a_cookie_lookup_or_an_answer_no_cookie_is_sent() {
    let server = Server::always(|| html("x"));
    fetch_source(&server.source("/"), &options()).await.unwrap();
    let (asked, log) = options_with_cookies(|_| None);
    fetch_source(&server.source("/"), &asked).await.unwrap();

    assert_eq!(log.lock().unwrap().len(), 1);
    for request in server.seen() {
        assert_eq!(request.header("cookie"), None);
    }
}

#[tokio::test]
async fn an_empty_user_agent_sends_none_and_an_invalid_one_is_an_error() {
    let server = Server::always(|| html("x"));
    let mut anonymous = options();
    anonymous.user_agent = String::new();
    fetch_source(&server.source("/"), &anonymous).await.unwrap();
    assert_eq!(server.seen()[0].header("user-agent"), None);

    let mut broken = options();
    broken.user_agent = "Mercury\r\nX-Injected: 1".to_owned();
    assert_eq!(
        fetch_source(&server.source("/"), &broken)
            .await
            .unwrap_err(),
        FetchError::Internal
    );
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn set_cookie_in_a_response_is_never_stored_or_sent_back() {
    let server = Server::start(|seen| match seen.path() {
        "/set" => Reply::Bytes(response(
            "302 Found",
            &[("Location", "/check"), ("Set-Cookie", "session=1; Path=/")],
            b"",
        )),
        _ => Reply::Bytes(response(
            "200 OK",
            &[
                ("Content-Type", "text/plain"),
                ("Set-Cookie", "again=2; Path=/"),
            ],
            b"checked",
        )),
    });
    let (options, _) = options_with_cookies(|_| None);

    fetch_source(&server.source("/set"), &options)
        .await
        .unwrap();
    fetch_source(&server.source("/check"), &options)
        .await
        .unwrap();

    let seen = server.seen();
    assert_eq!(seen.len(), 3);
    for request in &seen {
        assert_eq!(request.header("cookie"), None, "{}", request.request_line);
    }
}

// ---- redirects ----

#[tokio::test]
async fn a_redirect_chain_asks_for_cookies_at_every_hop() {
    let server = Server::start(|seen| match seen.path() {
        "/a" => redirect(301, "/b"),
        "/b" => redirect(302, "/c?x=1"),
        "/c" => redirect(303, "d"),
        "/d" => redirect(307, "/e"),
        "/e" => redirect(308, "/f/g"),
        _ => html("the end"),
    });
    let port = server.addr.port();
    let (options, asked) = options_with_cookies(|url| Some(format!("hop={}", url.path())));

    let document = fetch_source(&server.source("/a"), &options).await.unwrap();

    assert_eq!(document.final_url, server.url("/f/g"));
    assert_eq!(document.status, 200);
    assert_eq!(text_of(&document), "the end");
    let asked: Vec<String> = asked.lock().unwrap().iter().map(Url::to_string).collect();
    let expected: Vec<String> = ["/a", "/b", "/c?x=1", "/d", "/e", "/f/g"]
        .iter()
        .map(|path| format!("http://127.0.0.1:{port}{path}"))
        .collect();
    assert_eq!(asked, expected);
    let sent: Vec<(String, Option<String>)> = server
        .seen()
        .iter()
        .map(|request| {
            (
                request.target().to_owned(),
                request.header("cookie").map(str::to_owned),
            )
        })
        .collect();
    let expected: Vec<(String, Option<String>)> = [
        ("/a", "hop=/a"),
        ("/b", "hop=/b"),
        ("/c?x=1", "hop=/c"),
        ("/d", "hop=/d"),
        ("/e", "hop=/e"),
        ("/f/g", "hop=/f/g"),
    ]
    .iter()
    .map(|(target, cookie)| ((*target).to_owned(), Some((*cookie).to_owned())))
    .collect();
    assert_eq!(sent, expected);
}

#[tokio::test]
async fn a_redirect_to_another_host_gets_none_of_the_first_hosts_cookies() {
    let second = Server::always(|| html("landed"));
    let second_port = second.addr.port();
    let first =
        Server::always(move || redirect(302, &format!("http://127.0.0.1:{second_port}/landing")));
    let (options, asked) = options_with_cookies(move |url| {
        (url.port() != Some(second_port)).then(|| "session=first".to_owned())
    });

    let document = fetch_source(&first.source("/start"), &options)
        .await
        .unwrap();

    assert_eq!(document.final_url, second.url("/landing"));
    let asked: Vec<Option<u16>> = asked.lock().unwrap().iter().map(Url::port).collect();
    assert_eq!(asked, [Some(first.addr.port()), Some(second_port)]);
    assert_eq!(first.seen()[0].header("cookie"), Some("session=first"));
    assert_eq!(second.seen()[0].header("cookie"), None);
}

#[tokio::test]
async fn a_chain_of_exactly_the_most_redirects_is_followed() {
    let server = chain_of(MAX_REDIRECTS);
    let document = fetch_source(&server.source("/r/0"), &options())
        .await
        .unwrap();
    assert_eq!(text_of(&document), "end");
    assert_eq!(server.seen().len(), MAX_REDIRECTS + 1);
}

#[tokio::test]
async fn one_redirect_more_than_the_most_is_too_many() {
    let server = chain_of(MAX_REDIRECTS + 1);
    assert_eq!(
        fetch_source(&server.source("/r/0"), &options())
            .await
            .unwrap_err(),
        FetchError::TooManyRedirects
    );
    assert_eq!(server.seen().len(), MAX_REDIRECTS + 1);
}

/// `/r/0` redirects to `/r/1` and so on; `/r/<redirects>` answers.
fn chain_of(redirects: usize) -> Server {
    Server::start(move |seen| {
        let index: usize = seen
            .path()
            .trim_start_matches("/r/")
            .parse()
            .unwrap_or_default();
        if index < redirects {
            redirect(302, &format!("/r/{}", index + 1))
        } else {
            html("end")
        }
    })
}

#[tokio::test]
async fn a_redirect_loop_ends_with_too_many_redirects() {
    let server = Server::always(|| redirect(302, "/again"));
    assert_eq!(
        fetch_source(&server.source("/again"), &options())
            .await
            .unwrap_err(),
        FetchError::TooManyRedirects
    );
    assert_eq!(server.seen().len(), MAX_REDIRECTS + 1);
}

#[tokio::test]
async fn redirects_to_other_schemes_or_nowhere_are_bad_redirects() {
    for location in [
        "file:///etc/hosts",
        "javascript:alert(1)",
        "data:text/html,hi",
        "ftp://127.0.0.1/",
        "about:blank",
        "view-source:http://127.0.0.1/",
        "http://",
        "https://[::1",
    ] {
        let server = Server::always(move || redirect(302, location));
        assert_eq!(
            fetch_source(&server.source("/"), &options())
                .await
                .unwrap_err(),
            FetchError::BadRedirect,
            "{location}"
        );
        assert_eq!(server.seen().len(), 1, "{location}");
    }
    let no_location = Server::always(|| Reply::Bytes(response("302 Found", &[], b"")));
    assert_eq!(
        fetch_source(&no_location.source("/"), &options())
            .await
            .unwrap_err(),
        FetchError::BadRedirect
    );
}

#[tokio::test]
async fn credentials_in_a_redirect_are_dropped_not_sent() {
    let second = Server::always(|| html("landed"));
    let second_port = second.addr.port();
    let first = Server::always(move || {
        redirect(
            302,
            &format!("http://user:secret@127.0.0.1:{second_port}/x#frag"),
        )
    });

    let document = fetch_source(&first.source("/"), &options()).await.unwrap();

    assert_eq!(document.final_url, second.url("/x"));
    let landing = &second.seen()[0];
    assert_eq!(landing.header("authorization"), None);
    assert_eq!(landing.request_line, "GET /x HTTP/1.1");
}

// ---- failures ----

#[tokio::test]
async fn a_refused_connection_is_unreachable() {
    let url = source(&format!("http://127.0.0.1:{}/", closed_port()));
    assert_eq!(
        fetch_source(&url, &options()).await.unwrap_err(),
        FetchError::Unreachable
    );
}

#[tokio::test]
async fn a_server_that_never_answers_times_out_within_the_bound() {
    let server = Server::always(|| Reply::Stall);
    let mut options = options();
    options.total_timeout = Duration::from_millis(400);
    let started = Instant::now();

    let error = fetch_source(&server.source("/"), &options)
        .await
        .unwrap_err();

    assert_eq!(error, FetchError::Timeout);
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(350), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

#[tokio::test]
async fn a_server_that_stops_in_the_middle_of_the_body_times_out() {
    let head = response(
        "200 OK",
        &[("Content-Type", "text/plain"), ("Content-Length", "100000")],
        b"the first few bytes",
    );
    let server = Server::always(move || Reply::ThenHold(head.clone()));
    let mut options = options();
    options.total_timeout = Duration::from_millis(400);
    let started = Instant::now();

    let error = fetch_source(&server.source("/"), &options)
        .await
        .unwrap_err();

    assert_eq!(error, FetchError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(server.wait_for_hangups(1, Duration::from_secs(5)).await);
}

#[tokio::test]
async fn a_secure_connection_to_a_server_that_does_not_speak_tls_is_a_tls_failure() {
    let server = Server::raw(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    let url = source(&format!("https://127.0.0.1:{}/", server.addr.port()));

    assert_eq!(
        fetch_source(&url, &options()).await.unwrap_err(),
        FetchError::Tls
    );
    assert_eq!(server.connections(), 1);
}

// ---- the browser's rules for an address (HTTPS-only, HSTS, bad ports) ----

/// A policy like the engine glue's: `http` becomes `https` for these hosts (HSTS, HTTPS-only),
/// and these ports are refused (the bad-port block).
fn policy(upgrade_ports: Vec<u16>, refuse_ports: Vec<u16>) -> HopPolicy {
    Arc::new(move |url: &Url| {
        if url.port().is_some_and(|port| refuse_ports.contains(&port)) {
            return HopVerdict::Refuse;
        }
        let mut go = url.clone();
        if url.scheme() == "http" && url.port().is_some_and(|port| upgrade_ports.contains(&port)) {
            go.set_scheme("https").expect("http to https");
        }
        HopVerdict::Go(go)
    })
}

#[tokio::test]
async fn an_address_the_rules_upgrade_is_fetched_over_https_and_never_in_the_clear() {
    // The server does not speak TLS: a request to it as `https` fails, and what it reads first is
    // a TLS hello (record type 0x16), not `GET`. The first address is typed as `http://`.
    let server = Server::raw(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    let port = server.addr.port();
    let (mut options, asked) = options_with_cookies(|_| Some("sid=abc".to_owned()));
    options.policy = Some(policy(vec![port], vec![]));

    let outcome = fetch_source(&server.source("/"), &options).await;

    assert_eq!(outcome.unwrap_err(), FetchError::Tls);
    let first = server.first_bytes();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].first(), Some(&0x16), "a TLS hello, not a GET");
    // The cookies were asked for the address that would be requested, not the typed one, and
    // after the rules: a cookie of the `http` address is never looked up.
    let asked: Vec<String> = asked
        .lock()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(asked, [format!("https://127.0.0.1:{port}/")]);
}

#[tokio::test]
async fn an_address_without_a_rule_is_fetched_as_typed() {
    let server = Server::always(|| html("as typed"));
    let mut options = options();
    options.policy = Some(policy(vec![], vec![]));

    let document = fetch_source(&server.source("/"), &options).await.unwrap();

    assert_eq!(text_of(&document), "as typed");
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn a_blocked_port_is_refused_before_any_connection_or_cookie_lookup() {
    // Ports of mail, SSH and printers: a GET there is a cross-protocol request the engine forbids.
    for port in [25_u16, 22, 9100] {
        let (mut options, asked) = options_with_cookies(|_| Some("sid=abc".to_owned()));
        options.policy = Some(policy(vec![], vec![port]));
        let url = source(&format!("http://127.0.0.1:{port}/"));

        assert_eq!(
            fetch_source(&url, &options).await.unwrap_err(),
            FetchError::Blocked,
            "port {port}"
        );
        assert!(asked.lock().unwrap().is_empty(), "port {port}");
    }
}

#[tokio::test]
async fn a_redirect_to_a_blocked_address_is_refused_and_nothing_is_sent_there() {
    let first = Server::always(|| redirect(302, "http://127.0.0.1:25/inside"));
    let (mut options, asked) = options_with_cookies(|_| Some("sid=abc".to_owned()));
    options.policy = Some(policy(vec![], vec![25]));

    let outcome = fetch_source(&first.source("/start"), &options).await;

    assert_eq!(outcome.unwrap_err(), FetchError::Blocked);
    assert_eq!(first.seen().len(), 1, "the first hop was requested");
    let asked: Vec<Option<u16>> = asked.lock().unwrap().iter().map(Url::port).collect();
    assert_eq!(
        asked,
        [Some(first.addr.port())],
        "no cookies were looked up for the refused hop"
    );
}

#[tokio::test]
async fn a_redirect_to_http_is_upgraded_like_the_first_address() {
    let second = Server::raw(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    let second_port = second.addr.port();
    let first =
        Server::always(move || redirect(302, &format!("http://127.0.0.1:{second_port}/landing")));
    let mut options = options();
    options.policy = Some(policy(vec![second_port], vec![]));

    let outcome = fetch_source(&first.source("/start"), &options).await;

    assert_eq!(outcome.unwrap_err(), FetchError::Tls);
    let hello = second.first_bytes();
    assert_eq!(hello.len(), 1);
    assert_eq!(
        hello[0].first(),
        Some(&0x16),
        "the second hop was tried over TLS, never in the clear"
    );
}

#[tokio::test]
async fn a_rule_that_answers_with_something_else_than_a_web_address_is_a_refusal() {
    let server = Server::always(|| html("x"));
    let mut options = options();
    options.policy = Some(Arc::new(|_: &Url| {
        HopVerdict::Go(Url::parse("file:///etc/hosts").unwrap())
    }));

    assert_eq!(
        fetch_source(&server.source("/"), &options)
            .await
            .unwrap_err(),
        FetchError::Blocked
    );
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn the_language_of_the_browser_is_sent_and_en_us_only_without_one() {
    let server = Server::always(|| html("x"));
    let mut options = options();
    options.accept_language = "de-CH".to_owned();

    fetch_source(&server.source("/"), &options).await.unwrap();
    assert_eq!(server.seen()[0].header("accept-language"), Some("de-CH"));

    options.accept_language = "not valid\nat all".to_owned();
    assert_eq!(
        fetch_source(&server.source("/"), &options)
            .await
            .unwrap_err(),
        FetchError::Internal
    );
}

#[tokio::test]
async fn dropping_the_future_closes_the_connection() {
    let head = response(
        "200 OK",
        &[
            ("Content-Type", "text/plain"),
            ("Content-Length", "1000000"),
        ],
        &[b'a'; 100],
    );
    let server = Server::always(move || Reply::ThenHold(head.clone()));
    let url = server.source("/");
    let options = options();

    // The fetch has 20 s; the caller gives up after a third of a second and drops it.
    let outcome =
        tokio::time::timeout(Duration::from_millis(300), fetch_source(&url, &options)).await;

    assert!(
        outcome.is_err(),
        "the fetch should still be waiting for the body"
    );
    assert!(
        server.wait_for_hangups(1, Duration::from_secs(5)).await,
        "the server sees the connection close"
    );
}

// ---- the proxy ----

/// A proxy that answers an absolute-form request with a page naming the target it was asked for,
/// and refuses `CONNECT`.
fn forwarding_proxy() -> Server {
    Server::start(|seen| {
        if seen.request_line.starts_with("GET ") {
            html("<p>via proxy</p>")
        } else {
            Reply::Bytes(response("403 Forbidden", &[], b"no tunnels"))
        }
    })
}

#[tokio::test]
async fn an_http_target_goes_through_the_http_proxy_with_the_target_as_host() {
    let proxy = forwarding_proxy();
    let mut options = options();
    options.proxy.http = proxy.proxy_uri();
    // The name is never resolved: the proxy gets the request.
    let url = source("http://target.test:8080/page?x=1");

    let document = fetch_source(&url, &options).await.unwrap();

    assert_eq!(text_of(&document), "<p>via proxy</p>");
    let seen = proxy.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].request_line,
        "GET http://target.test:8080/page?x=1 HTTP/1.1"
    );
    assert_eq!(seen[0].header("host"), Some("target.test:8080"));
    assert_eq!(seen[0].header("user-agent"), Some("MercuryTest/1.0"));
    assert_eq!(seen[0].header("accept"), Some(DOCUMENT_ACCEPT));
}

#[tokio::test]
async fn a_proxy_that_answers_with_an_error_page_gives_that_page_as_the_source() {
    let proxy = Server::always(|| {
        Reply::Bytes(response(
            "502 Bad Gateway",
            &[("Content-Type", "text/plain")],
            b"the proxy could not reach the origin",
        ))
    });
    let mut options = options();
    options.proxy.http = proxy.proxy_uri();

    let document = fetch_source(&source("http://target.test/"), &options)
        .await
        .unwrap();

    assert_eq!(document.status, 502);
    assert_eq!(text_of(&document), "the proxy could not reach the origin");
}

#[tokio::test]
async fn the_proxy_address_may_carry_credentials_and_may_leave_out_the_scheme() {
    let proxy = forwarding_proxy();
    let port = proxy.addr.port();

    let mut with_credentials = options();
    with_credentials.proxy.http = format!("http://user:pw@127.0.0.1:{port}");
    fetch_source(&source("http://target.test/"), &with_credentials)
        .await
        .unwrap();
    assert_eq!(
        proxy.seen()[0].header("proxy-authorization"),
        Some("Basic dXNlcjpwdw==")
    );

    let mut bare = options();
    bare.proxy.http = format!("127.0.0.1:{port}");
    fetch_source(&source("http://target.test/"), &bare)
        .await
        .unwrap();
    assert_eq!(proxy.seen().len(), 2);
}

#[tokio::test]
async fn no_proxy_sends_a_target_around_the_proxy() {
    for no_proxy in [
        "127.0.0.1",
        " 127.0.0.1 ",
        "127.0.0.0/8",
        "10.0.0.0/8, 127.0.0.1",
        "*.example, 127.0.0.1",
    ] {
        let target = Server::always(|| html("direct"));
        let proxy = forwarding_proxy();
        let mut options = options();
        options.proxy.http = proxy.proxy_uri();
        options.proxy.no_proxy = no_proxy.to_owned();

        let document = fetch_source(&target.source("/page"), &options)
            .await
            .unwrap();

        assert_eq!(text_of(&document), "direct", "{no_proxy:?}");
        assert_eq!(target.seen().len(), 1, "{no_proxy:?}");
        assert_eq!(proxy.connections(), 0, "{no_proxy:?}");
    }
}

#[tokio::test]
async fn a_domain_in_no_proxy_bypasses_the_proxy_for_its_names() {
    let target = Server::always(|| html("direct"));
    let proxy = forwarding_proxy();
    for no_proxy in ["localhost", ".localhost", "LOCALHOST"] {
        let mut options = options();
        options.proxy.http = proxy.proxy_uri();
        options.proxy.no_proxy = no_proxy.to_owned();
        let url = source(&format!("http://localhost:{}/", target.addr.port()));

        let document = fetch_source(&url, &options).await.unwrap();

        assert_eq!(text_of(&document), "direct", "{no_proxy:?}");
    }
    assert_eq!(proxy.connections(), 0);
}

#[tokio::test]
async fn an_empty_no_proxy_does_not_exempt_loopback() {
    // The engine's matcher has no built-in exceptions: with a proxy set and nothing listed, even
    // 127.0.0.1 goes through it. The view-source fetch does the same as the page did.
    let target = Server::always(|| html("direct"));
    let proxy = forwarding_proxy();
    let mut options = options();
    options.proxy.http = proxy.proxy_uri();

    let document = fetch_source(&target.source("/"), &options).await.unwrap();

    assert_eq!(text_of(&document), "<p>via proxy</p>");
    assert_eq!(target.connections(), 0);
}

#[tokio::test]
async fn a_scheme_without_a_proxy_connects_directly_as_the_engine_does() {
    let target = Server::always(|| html("direct"));
    let proxy = forwarding_proxy();
    let mut options = options();
    options.proxy.https = proxy.proxy_uri();

    let document = fetch_source(&target.source("/"), &options).await.unwrap();

    assert_eq!(text_of(&document), "direct");
    assert_eq!(proxy.connections(), 0);
}

#[tokio::test]
async fn an_https_target_asks_the_proxy_for_a_tunnel_and_a_refusal_is_a_proxy_error() {
    let target = Server::always(|| html("must not be reached"));
    let proxy = forwarding_proxy();
    let mut options = options();
    options.proxy.https = proxy.proxy_uri();
    let port = target.addr.port();
    let url = source(&format!("https://127.0.0.1:{port}/secret"));

    let error = fetch_source(&url, &options).await.unwrap_err();

    assert_eq!(error, FetchError::Proxy);
    let seen = proxy.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].request_line,
        format!("CONNECT 127.0.0.1:{port} HTTP/1.1")
    );
    assert_eq!(target.connections(), 0, "nothing went around the proxy");
}

#[tokio::test]
async fn an_unreachable_proxy_is_a_proxy_error_and_the_target_is_never_contacted() {
    let target = Server::always(|| html("must not be reached"));
    let dead = format!("http://127.0.0.1:{}", closed_port());

    let mut for_http = options();
    for_http.proxy.http = dead.clone();
    assert_eq!(
        fetch_source(&target.source("/"), &for_http)
            .await
            .unwrap_err(),
        FetchError::Proxy
    );

    let mut for_https = options();
    for_https.proxy.https = dead;
    let secure = source(&format!("https://127.0.0.1:{}/", target.addr.port()));
    assert_eq!(
        fetch_source(&secure, &for_https).await.unwrap_err(),
        FetchError::Proxy
    );

    assert_eq!(target.connections(), 0);
}

#[tokio::test]
async fn a_proxy_that_cannot_be_used_is_a_proxy_error_not_a_direct_connection() {
    let target = Server::always(|| html("must not be reached"));
    for broken in [
        "socks5://127.0.0.1:1080",
        "ftp://127.0.0.1:21",
        "http://",
        "http://[::1",
    ] {
        let mut options = options();
        options.proxy.http = broken.to_owned();
        assert_eq!(
            fetch_source(&target.source("/"), &options)
                .await
                .unwrap_err(),
            FetchError::Proxy,
            "{broken}"
        );
    }
    assert_eq!(target.connections(), 0);
}

#[tokio::test]
async fn each_hop_of_a_redirect_chain_is_matched_against_the_proxy_on_its_own() {
    // The first host is on the bypass list and is connected to directly; the redirect leads to a
    // name that is not, and that request goes to the proxy.
    let proxy = forwarding_proxy();
    let first = Server::always(|| redirect(302, "http://hop2.test:81/x"));
    let mut options = options();
    options.proxy.http = proxy.proxy_uri();
    options.proxy.no_proxy = "127.0.0.1".to_owned();

    let document = fetch_source(&first.source("/start"), &options)
        .await
        .unwrap();

    assert_eq!(first.seen().len(), 1);
    assert_eq!(proxy.seen().len(), 1);
    assert_eq!(
        proxy.seen()[0].request_line,
        "GET http://hop2.test:81/x HTTP/1.1"
    );
    assert_eq!(document.final_url.as_str(), "http://hop2.test:81/x");
    assert_eq!(text_of(&document), "<p>via proxy</p>");
}

// ---- the environment ----

const ENVIRONMENT_CHILD_TARGET: &str = "MERCURY_VIEW_SOURCE_TEST_DIRECT_TARGET";

/// The proxy variables of the environment are not read: the engine's three values are the only
/// source. This test runs the test below in a child process whose environment names a proxy.
#[tokio::test]
async fn environment_proxy_variables_are_not_used() {
    let target = Server::always(|| html("direct"));
    let trap = Server::always(|| html("through the proxy of the environment"));
    let executable = std::env::current_exe().expect("the test binary");

    let mut command = std::process::Command::new(executable);
    command
        .args([
            "--exact",
            "environment_proxy_variables_child",
            "--nocapture",
        ])
        .env(ENVIRONMENT_CHILD_TARGET, target.url("/").as_str())
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        command.env(name, trap.proxy_uri());
    }
    let output = command.output().expect("run the child test");

    assert!(
        output.status.success(),
        "the child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        trap.connections(),
        0,
        "the proxy of the environment was used"
    );
    assert_eq!(
        target.seen().len(),
        1,
        "the target was reached once, directly"
    );
}

/// The body of the child process of the test above; without the variable it does nothing.
#[tokio::test]
async fn environment_proxy_variables_child() {
    let Ok(target) = std::env::var(ENVIRONMENT_CHILD_TARGET) else {
        return;
    };
    let page = Url::parse(&target).expect("the target address");
    let url = ViewSourceUrl::for_page(&page).expect("a source address");
    let document = fetch_source(&url, &options()).await.expect("a document");
    assert_eq!(text_of(&document), "direct");
}
