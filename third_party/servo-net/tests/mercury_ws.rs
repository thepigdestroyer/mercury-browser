// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for WebSockets through the proxy and the network policy in the handshake (EC23a, EC5, EC6,
// EC25); see mercury_ws.rs.

//! WebSockets go through the proxy that HTTP uses, and the handshake follows the network policy.
//!
//! * `connect_tcp` is tested against a **fake `CONNECT` proxy** (a thread that records what it is asked and
//!   forwards the bytes) and a plain TCP target: the proxy sees `CONNECT 127.0.0.1:<port>`; with the host in
//!   `no_proxy` it sees nothing; with no proxy nothing changes; `wss` uses the HTTPS proxy preference and
//!   `ws` the HTTP one; the proxy's credentials are sent; a proxy that refuses fails the connection.
//! * the handshake request that `create_handshake_request` builds carries `Sec-GPC: 1` and, for a
//!   third-party socket, no `Cookie`; a first-party socket sends its cookie; `SameSite` rules apply; the
//!   policy off changes nothing.
//! * the whole path (`fetch` of a WebSocket request, `start_websocket`, a real server built with
//!   `async_tungstenite::tokio::accept_hdr_async`) shows what the server received, whether the socket went
//!   through the proxy, and which `Set-Cookie` of the answer was stored.
//!
//! The proxy preferences are global to the process. The tests that set them hold [`PROXY_PREFS`], and the
//! `connect_tcp` tests read them when the function is **called** (so the window in which another test could
//! notice them is as short as the call) and put them back. The fake proxy forwards any `CONNECT`, so a test
//! that happens to build an HTTP client in that window is still served.

use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener as StdTcpListener, TcpStream as StdTcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::thread;
use std::time::Duration;

use async_tungstenite::tokio::accept_hdr_async;
use content_security_policy as csp;
use futures::StreamExt;
use net::async_runtime::{spawn_blocking_task, spawn_task};
use net::cookie::ServoCookie;
use net::fetch::methods::{self, FetchContext, WebSocketChannel};
use net::mercury_ws::{connect_tcp, create_handshake_request, destination_for};
use net::test::HttpState;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::policy_container::PolicyContainer;
use net_traits::pub_domains::Site;
use net_traits::request::{
    CacheMode, CredentialsMode, InsecureRequestsPolicy, Origin, RedirectMode, Referrer, Request,
    RequestBuilder, RequestClient, RequestMode, ServiceWorkersMode,
};
use net_traits::response::Response;
use net_traits::{CookieSource, FetchTaskTarget, WebSocketDomAction, WebSocketNetworkEvent};
use parking_lot::Mutex;
use profile_traits::generic_callback::GenericCallback as ProfileGenericCallback;
use servo_base::generic_channel::{LazyCallback, SendError, lazy_callback};
use servo_base::id::{TEST_PIPELINE_ID, TEST_WEBVIEW_ID};
use servo_config::prefs::{self, Preferences};
use servo_url::{ImmutableOrigin, ServoUrl};
use tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};

use crate::{create_http_state, new_fetch_context};

// ---- the proxy preferences -------------------------------------------------------------------------------

static PROXY_PREFS: StdMutex<()> = StdMutex::new(());

/// The proxy preferences, set until the guard is dropped (which puts the old ones back). Holds
/// [`PROXY_PREFS`], so two such guards never overlap.
struct ProxyPrefsGuard {
    saved: Preferences,
    _lock: MutexGuard<'static, ()>,
}

impl ProxyPrefsGuard {
    fn set(http: &str, https: &str, no_proxy: &str) -> ProxyPrefsGuard {
        let lock = PROXY_PREFS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The read guard of `get()` is a temporary of this statement: `set` below takes the write lock.
        let saved = prefs::get().clone();
        let mut changed = saved.clone();
        changed.network_http_proxy_uri = http.to_owned();
        changed.network_https_proxy_uri = https.to_owned();
        changed.network_http_no_proxy = no_proxy.to_owned();
        prefs::set(changed);
        ProxyPrefsGuard { saved, _lock: lock }
    }
}

impl Drop for ProxyPrefsGuard {
    fn drop(&mut self) {
        prefs::set(self.saved.clone());
    }
}

/// Run `build` with the proxy preferences set, then put them back. `build` must read them synchronously, as
/// `connect_tcp` does when it is called.
fn with_proxy_prefs<R>(http: &str, https: &str, no_proxy: &str, build: impl FnOnce() -> R) -> R {
    let _guard = ProxyPrefsGuard::set(http, https, no_proxy);
    build()
}

// ---- a fake CONNECT proxy ---------------------------------------------------------------------------------

/// One request that the proxy received.
#[derive(Clone, Debug)]
struct Seen {
    /// `CONNECT 127.0.0.1:1234 HTTP/1.1`
    request_line: String,
    /// The target of the request line, `127.0.0.1:1234`.
    target: String,
    /// The header lines, as received.
    headers: Vec<String>,
}

struct FakeProxy {
    port: u16,
    seen: Arc<StdMutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
}

impl FakeProxy {
    /// A proxy that forwards every `CONNECT`, or, with `refuse`, answers every request with a 403.
    fn start(refuse: bool) -> FakeProxy {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let seen = seen.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                for connection in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(connection) = connection else {
                        break;
                    };
                    let seen = seen.clone();
                    thread::spawn(move || serve_connect(connection, seen, refuse));
                }
            });
        }
        FakeProxy { port, seen, stop }
    }

    /// The value of a proxy preference that points at this proxy.
    fn uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn uri_with_credentials(&self, user: &str, password: &str) -> String {
        format!("http://{user}:{password}@127.0.0.1:{}", self.port)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Every `CONNECT` target the proxy was asked for.
    fn targets(&self) -> Vec<String> {
        self.seen().into_iter().map(|seen| seen.target).collect()
    }

    /// How many times the proxy was asked for `127.0.0.1:<port>`.
    fn tunnels_to(&self, port: u16) -> usize {
        let wanted = format!("127.0.0.1:{port}");
        self.targets()
            .iter()
            .filter(|target| **target == wanted)
            .count()
    }
}

impl Drop for FakeProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so that its thread ends.
        let _ = StdTcpStream::connect(("127.0.0.1", self.port));
    }
}

/// The head of an HTTP message (up to the blank line), or `None`.
fn read_head(stream: &mut StdTcpStream) -> Option<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 || stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    Some(String::from_utf8_lossy(&head).into_owned())
}

fn serve_connect(mut client: StdTcpStream, seen: Arc<StdMutex<Vec<Seen>>>, refuse: bool) {
    let Some(head) = read_head(&mut client) else {
        return;
    };
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let headers: Vec<String> = lines
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    let mut words = request_line.split(' ');
    let method = words.next().unwrap_or_default().to_owned();
    let target = words.next().unwrap_or_default().to_owned();
    seen.lock().unwrap().push(Seen {
        request_line,
        target: target.clone(),
        headers,
    });

    if refuse || method != "CONNECT" {
        let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let Ok(upstream) = StdTcpStream::connect(target.as_str()) else {
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    if client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .is_err()
    {
        return;
    }
    // From here on both ends are plain byte streams.
    let _ = client.set_read_timeout(None);
    let (Ok(mut client_reader), Ok(mut upstream_writer)) =
        (client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    thread::spawn(move || {
        let _ = io::copy(&mut client_reader, &mut upstream_writer);
        let _ = upstream_writer.shutdown(Shutdown::Write);
    });
    let (mut upstream_reader, mut client_writer) = (upstream, client);
    let _ = io::copy(&mut upstream_reader, &mut client_writer);
    let _ = client_writer.shutdown(Shutdown::Write);
}

// ---- a plain TCP target -----------------------------------------------------------------------------------

/// A server that answers `ping` with `pong`, once per connection. Returns its port.
fn ping_pong_server() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut connection) = connection else {
                break;
            };
            thread::spawn(move || {
                let _ = connection.set_read_timeout(Some(Duration::from_secs(5)));
                let mut ping = [0u8; 4];
                if connection.read_exact(&mut ping).is_ok() && &ping == b"ping" {
                    let _ = connection.write_all(b"pong");
                }
            });
        }
    });
    port
}

/// Exchange `ping` for `pong` over the connection that `future` (a `connect_tcp` call) opens.
fn ping_pong(future: impl Future<Output = io::Result<tokio::net::TcpStream>>) -> io::Result<()> {
    let stream = spawn_blocking_task(future)?;
    let mut stream = stream.into_std()?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(b"ping")?;
    let mut pong = [0u8; 4];
    stream.read_exact(&mut pong)?;
    if &pong == b"pong" {
        Ok(())
    } else {
        Err(io::Error::other("not a pong"))
    }
}

/// Make sure that the runtime that `connect_tcp` and the servers run on exists (it is created with the first
/// embedder proxy).
fn init_runtime() {
    let _ = net::test_util::create_embedder_proxy();
}

// ---- connect_tcp -------------------------------------------------------------------------------------------

#[test]
fn the_destination_says_which_proxy_preference_applies() {
    assert_eq!(
        destination_for("127.0.0.1", 8080, false)
            .unwrap()
            .to_string(),
        "http://127.0.0.1:8080/"
    );
    assert_eq!(
        destination_for("example.com", 443, true)
            .unwrap()
            .to_string(),
        "https://example.com:443/"
    );
    // A bare IPv6 host gets its brackets; one that has them keeps them.
    assert_eq!(
        destination_for("::1", 9000, false).unwrap().to_string(),
        "http://[::1]:9000/"
    );
    assert_eq!(
        destination_for("[::1]", 9000, true).unwrap().to_string(),
        "https://[::1]:9000/"
    );
    // Not a host.
    let error = destination_for("not a host", 80, false).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn without_a_proxy_the_connection_is_direct() {
    init_runtime();
    let target = ping_pong_server();
    let future = with_proxy_prefs("", "", "", || connect_tcp("127.0.0.1", target, false));
    ping_pong(future).expect("a direct connection works as before");
}

#[test]
fn with_a_proxy_the_connection_is_a_connect_tunnel() {
    init_runtime();
    let target = ping_pong_server();
    let proxy = FakeProxy::start(false);
    let future = with_proxy_prefs(&proxy.uri(), "", "", || {
        connect_tcp("127.0.0.1", target, false)
    });
    ping_pong(future).expect("bytes go through the tunnel");

    let seen = proxy.seen();
    assert!(
        seen.iter()
            .any(|seen| seen.request_line == format!("CONNECT 127.0.0.1:{target} HTTP/1.1")),
        "the proxy saw the CONNECT: {seen:?}"
    );
}

#[test]
fn a_host_on_the_no_proxy_list_is_not_sent_to_the_proxy() {
    init_runtime();
    let target = ping_pong_server();
    let proxy = FakeProxy::start(false);
    let future = with_proxy_prefs(&proxy.uri(), "", "127.0.0.1", || {
        connect_tcp("127.0.0.1", target, false)
    });
    ping_pong(future).expect("a direct connection");
    assert_eq!(
        proxy.tunnels_to(target),
        0,
        "the proxy must not see a no_proxy host: {:?}",
        proxy.seen()
    );
}

#[test]
fn wss_uses_the_https_proxy_and_ws_the_http_proxy() {
    init_runtime();
    let target = ping_pong_server();

    // Only the HTTPS proxy is set: a `wss` socket goes through it, a `ws` socket does not.
    let https_proxy = FakeProxy::start(false);
    let future = with_proxy_prefs("", &https_proxy.uri(), "", || {
        connect_tcp("127.0.0.1", target, true)
    });
    ping_pong(future).expect("wss through the https proxy");
    assert_eq!(
        https_proxy.tunnels_to(target),
        1,
        "{:?}",
        https_proxy.seen()
    );

    let future = with_proxy_prefs("", &https_proxy.uri(), "", || {
        connect_tcp("127.0.0.1", target, false)
    });
    ping_pong(future).expect("ws goes direct");
    assert_eq!(
        https_proxy.tunnels_to(target),
        1,
        "a ws socket must not use the https proxy: {:?}",
        https_proxy.seen()
    );

    // And the other way round: only the HTTP proxy is set.
    let http_proxy = FakeProxy::start(false);
    let future = with_proxy_prefs(&http_proxy.uri(), "", "", || {
        connect_tcp("127.0.0.1", target, true)
    });
    ping_pong(future).expect("wss goes direct");
    assert_eq!(
        http_proxy.tunnels_to(target),
        0,
        "a wss socket must not use the http proxy: {:?}",
        http_proxy.seen()
    );
}

#[test]
fn the_proxy_gets_the_credentials_from_its_address() {
    init_runtime();
    let target = ping_pong_server();
    let proxy = FakeProxy::start(false);
    let uri = proxy.uri_with_credentials("alice", "secret");
    let future = with_proxy_prefs(&uri, "", "", || connect_tcp("127.0.0.1", target, false));
    ping_pong(future).expect("through the authenticating proxy");

    // base64("alice:secret")
    let seen = proxy.seen();
    assert!(
        seen.iter().any(|seen| seen.headers.iter().any(
            |header| header.eq_ignore_ascii_case("proxy-authorization: Basic YWxpY2U6c2VjcmV0")
        )),
        "the CONNECT carries the proxy's basic credentials: {seen:?}"
    );
}

#[test]
fn a_proxy_that_refuses_the_tunnel_fails_the_connection() {
    init_runtime();
    let target = ping_pong_server();
    let proxy = FakeProxy::start(true);
    let future = with_proxy_prefs(&proxy.uri(), "", "", || {
        connect_tcp("127.0.0.1", target, false)
    });
    let error = spawn_blocking_task(future).expect_err("a 403 to CONNECT is a failure");
    assert!(!error.to_string().is_empty());
    assert_eq!(proxy.tunnels_to(target), 1);
}

#[test]
fn an_address_that_is_not_a_host_fails_before_any_connection() {
    init_runtime();
    let future = with_proxy_prefs("", "", "", || connect_tcp("no host", 80, false));
    let error = spawn_blocking_task(future).expect_err("not a host");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

// ---- the handshake request ---------------------------------------------------------------------------------

fn origin_of(url: &str) -> ImmutableOrigin {
    ServoUrl::parse(url).unwrap().origin()
}

/// The client of a document at `page` whose top-level page is `top_level`.
fn client(top_level: &str, page: &str) -> RequestClient {
    RequestClient {
        preloaded_resources: Default::default(),
        policy_container: Arc::new(PolicyContainer::default()),
        origin: Origin::Origin(origin_of(page)),
        is_nested_browsing_context: top_level != page,
        insecure_requests_policy: InsecureRequestsPolicy::DoNotUpgrade,
        has_trustworthy_ancestor_origin: false,
        network_partition_key: Some(Site::from_origin(&origin_of(top_level))),
        browser_initiated_navigation: false,
    }
}

/// The builder `document/websocket.rs` makes (with the URL scheme already `http`, as `resource_thread.rs`
/// leaves it): a socket to `ws_url`, opened by the document at `page` of a page whose top level is
/// `top_level`. `None` for `top_level` is a request without a client, whose top-level site nobody knows.
fn socket_builder(ws_url: &str, page: &str, top_level: Option<&str>) -> RequestBuilder {
    let ws_url = ServoUrl::parse(ws_url).unwrap();
    let mut http_url = ws_url.clone();
    http_url.as_mut_url().set_scheme("http").unwrap();
    let mut builder = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::from_url_without_having_claimed_blob(http_url),
        Referrer::NoReferrer,
    )
    .mode(RequestMode::WebSocket {
        protocols: vec![],
        original_url: ws_url,
    })
    .service_workers_mode(ServiceWorkersMode::None)
    .credentials_mode(CredentialsMode::Include)
    .cache_mode(CacheMode::NoCache)
    .redirect_mode(RedirectMode::Error)
    .pipeline_id(Some(TEST_PIPELINE_ID))
    .origin(origin_of(page));
    if let Some(top_level) = top_level {
        builder = builder.client(client(top_level, page));
    }
    builder
}

fn store_cookie(state: &HttpState, url: &str, cookie: &str) {
    let url = ServoUrl::parse(url).unwrap();
    let cookie = ServoCookie::from_cookie_string(cookie, &url, CookieSource::HTTP)
        .unwrap_or_else(|| panic!("a cookie: {cookie}"));
    state
        .cookie_jar
        .write()
        .push(cookie, &url, CookieSource::HTTP);
}

/// The policy as Mercury runs it: third-party cookies blocked, Global Privacy Control on.
fn mercury_policy(state: &HttpState) {
    state.net_policy.set_block_third_party_cookies(true);
    state.net_policy.set_global_privacy_control(true);
}

fn handshake(state: &Arc<HttpState>, builder: RequestBuilder) -> Request {
    create_handshake_request(builder, state.clone()).expect("a handshake request")
}

/// The names of the cookies in a request's `Cookie` header, sorted.
fn cookie_names(request: &Request) -> Vec<String> {
    let mut names: Vec<String> = request
        .headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split("; ")
                .map(|pair| pair.split('=').next().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn sec_gpc(request: &Request) -> Option<String> {
    request
        .headers
        .get("sec-gpc")
        .map(|value| value.to_str().unwrap().to_owned())
}

const SERVER: &str = "http://127.0.0.1:4242/";
const WS: &str = "ws://127.0.0.1:4242/";

#[test]
fn a_third_party_socket_sends_no_cookie_and_says_sec_gpc() {
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    store_cookie(&state, SERVER, "session=abc");

    // The page is on other.test; the socket goes to 127.0.0.1.
    let request = handshake(
        &state,
        socket_builder(WS, "http://other.test", Some("http://other.test")),
    );
    assert!(request.headers.get("cookie").is_none(), "no Cookie header");
    assert_eq!(sec_gpc(&request).as_deref(), Some("1"));
    assert_eq!(state.net_policy.stats().third_party_cookies_blocked, 1);
}

#[test]
fn a_first_party_socket_sends_its_cookie_and_says_sec_gpc() {
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    store_cookie(&state, SERVER, "session=abc");

    let request = handshake(
        &state,
        socket_builder(WS, "http://127.0.0.1:4242", Some("http://127.0.0.1:4242")),
    );
    assert_eq!(cookie_names(&request), vec!["session".to_owned()]);
    assert_eq!(sec_gpc(&request).as_deref(), Some("1"));
}

#[test]
fn a_socket_whose_top_level_site_nobody_knows_is_third_party() {
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    store_cookie(&state, SERVER, "session=abc");

    let request = handshake(&state, socket_builder(WS, "http://127.0.0.1:4242", None));
    assert!(request.headers.get("cookie").is_none(), "fail closed");
    assert_eq!(sec_gpc(&request).as_deref(), Some("1"));
}

#[test]
fn a_site_the_user_allowed_has_its_third_party_sockets_cookies() {
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    store_cookie(&state, SERVER, "session=abc");
    state
        .net_policy
        .allow_third_party_cookies_on("other.test".to_owned(), true);

    let request = handshake(
        &state,
        socket_builder(WS, "http://other.test", Some("http://other.test")),
    );
    assert_eq!(cookie_names(&request), vec!["session".to_owned()]);
}

#[test]
fn without_the_policy_the_handshake_is_what_servo_builds() {
    // NetPolicy::new(): nothing blocked, no Sec-GPC.
    let state = Arc::new(create_http_state(None));
    assert!(!state.net_policy.block_third_party_cookies());
    assert!(!state.net_policy.global_privacy_control());
    store_cookie(&state, SERVER, "session=abc");

    let request = handshake(
        &state,
        socket_builder(WS, "http://other.test", Some("http://other.test")),
    );
    assert_eq!(cookie_names(&request), vec!["session".to_owned()]);
    assert_eq!(sec_gpc(&request), None);
}

#[test]
fn global_privacy_control_off_means_no_header_even_when_cookies_are_blocked() {
    let state = Arc::new(create_http_state(None));
    state.net_policy.set_block_third_party_cookies(true);
    store_cookie(&state, SERVER, "session=abc");

    let request = handshake(
        &state,
        socket_builder(WS, "http://other.test", Some("http://other.test")),
    );
    assert_eq!(sec_gpc(&request), None);
    assert!(request.headers.get("cookie").is_none());
}

#[test]
fn the_policy_of_one_state_does_not_leak_into_another() {
    let first = Arc::new(create_http_state(None));
    let second = Arc::new(create_http_state(None));
    mercury_policy(&first);
    assert!(!second.net_policy.is_same_policy(&first.net_policy));
    store_cookie(&second, SERVER, "session=abc");

    let request = handshake(
        &second,
        socket_builder(WS, "http://other.test", Some("http://other.test")),
    );
    assert_eq!(cookie_names(&request), vec!["session".to_owned()]);
    assert_eq!(sec_gpc(&request), None);
}

#[test]
fn samesite_cookies_are_not_sent_to_a_socket_opened_by_a_frame_of_another_site() {
    // The top-level page and the socket are one site (127.0.0.1), so third-party blocking lets cookies
    // through; but a frame of other.test opened the socket, so it is not a same-site request.
    let state = Arc::new(create_http_state(None));
    state.net_policy.set_block_third_party_cookies(true);
    state.net_policy.set_same_site_enforcement(true);
    store_cookie(&state, SERVER, "strict=1; SameSite=Strict");
    store_cookie(&state, SERVER, "lax=1; SameSite=Lax");
    store_cookie(&state, SERVER, "plain=1");

    let from_frame = handshake(
        &state,
        socket_builder(WS, "http://other.test", Some("http://127.0.0.1:4242")),
    );
    assert_eq!(cookie_names(&from_frame), vec!["plain".to_owned()]);

    // The page itself opens the socket: every cookie goes.
    let from_page = handshake(
        &state,
        socket_builder(WS, "http://127.0.0.1:4242", Some("http://127.0.0.1:4242")),
    );
    assert_eq!(
        cookie_names(&from_page),
        vec!["lax".to_owned(), "plain".to_owned(), "strict".to_owned()]
    );
}

// ---- the whole path ----------------------------------------------------------------------------------------

/// What the WebSocket server received in each handshake (header names in lower case).
type Handshakes = Arc<StdMutex<Vec<HashMap<String, String>>>>;

/// The callback of the server side of a handshake: it records the request headers and adds `Set-Cookie` to
/// the answer when asked to.
fn record_handshake(
    seen: Handshakes,
    set_cookie: Option<&'static str>,
) -> impl FnOnce(&HandshakeRequest, HandshakeResponse) -> Result<HandshakeResponse, ErrorResponse> + Unpin
{
    move |request: &HandshakeRequest, mut response: HandshakeResponse| {
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        seen.lock().unwrap().push(headers);
        if let Some(cookie) = set_cookie {
            response
                .headers_mut()
                .append("Set-Cookie", cookie.parse().unwrap());
        }
        Ok(response)
    }
}

/// A WebSocket server on 127.0.0.1 (the port is returned). It records every handshake, answers it with
/// `Set-Cookie: <set_cookie>` when given, and keeps the connection open until the client closes it.
fn websocket_server(set_cookie: Option<&'static str>) -> (u16, Handshakes) {
    init_runtime();
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let listener =
        spawn_blocking_task(async move { tokio::net::TcpListener::from_std(listener).unwrap() });
    let handshakes: Handshakes = Arc::new(StdMutex::new(Vec::new()));
    let seen = handshakes.clone();
    spawn_task(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let seen = seen.clone();
            spawn_task(async move {
                if let Ok(mut websocket) =
                    accept_hdr_async(stream, record_handshake(seen, set_cookie)).await
                {
                    while let Some(Ok(_message)) = websocket.next().await {}
                }
            });
        }
    });
    (port, handshakes)
}

/// A target for a fetch whose response nobody reads.
struct IgnoreResponse;

impl FetchTaskTarget for IgnoreResponse {
    fn process_request_body(&mut self, _: &Request) {}
    fn process_response(&mut self, _: &Request, _: &Response) {}
    fn process_response_chunk(&mut self, _: &Request, _: bytes::Bytes) {}
    fn process_response_eof(&mut self, _: &Request, _: &Response) {}
    fn process_csp_violations(&mut self, _: &Request, _: Vec<csp::Violation>) {}
    fn process_response_length_hint(&mut self, _: &Request, _: usize) {}
}

/// A socket opened through the real fetch.
struct Opened<G> {
    /// What the network thread told the DOM about the socket.
    events: Receiver<WebSocketNetworkEvent>,
    /// Keeps the DOM end of the socket open; dropping it closes the connection.
    _dom_actions: LazyCallback<WebSocketDomAction>,
    /// What `open_socket`'s `before` returned (the guard of the proxy preferences, say): it lives as long as
    /// the socket object does.
    _before: G,
}

/// Open the socket that `builder` describes with `fetch`, the way `resource_thread.rs` does: the handshake
/// request first, then the fetch, which ends in `start_websocket`. `before` runs just before the fetch is
/// started and what it returns is kept until the end of the test.
fn open_socket<G>(
    state: &Arc<HttpState>,
    builder: RequestBuilder,
    before: impl FnOnce() -> G,
) -> Opened<G> {
    let mut context: FetchContext = new_fetch_context(None, None);
    context.state = state.clone();

    let (event_sender, events) = channel();
    let event_callback =
        ProfileGenericCallback::new(move |event: Result<WebSocketNetworkEvent, SendError>| {
            if let Ok(event) = event {
                let _ = event_sender.send(event);
            }
        })
        .expect("a callback");
    let (dom_actions, dom_action_receiver) = lazy_callback::<WebSocketDomAction>();
    context.websocket_chan = Some(Arc::new(Mutex::new(WebSocketChannel::new(
        event_callback,
        Some(dom_action_receiver),
    ))));

    let request = create_handshake_request(builder, state.clone()).expect("a handshake request");
    let before = before();
    spawn_task(async move {
        let mut target = IgnoreResponse;
        methods::fetch(request, &mut target, &context).await;
    });
    Opened {
        events,
        _dom_actions: dom_actions,
        _before: before,
    }
}

/// Wait for the socket to open (or fail).
fn established<G>(opened: &Opened<G>) -> bool {
    match opened.events.recv_timeout(Duration::from_secs(15)) {
        Ok(WebSocketNetworkEvent::ConnectionEstablished { .. }) => true,
        Ok(other) => {
            eprintln!("the socket did not open: {other:?}");
            false
        }
        Err(error) => {
            eprintln!("no event from the socket: {error}");
            false
        }
    }
}

#[test]
fn a_websocket_goes_through_the_proxy_when_http_does() {
    let (port, handshakes) = websocket_server(None);
    let proxy = FakeProxy::start(false);
    let state = Arc::new(create_http_state(None));
    let url = format!("ws://127.0.0.1:{port}/");
    let page = format!("http://127.0.0.1:{port}");

    let proxy_uri = proxy.uri();
    let opened = open_socket(&state, socket_builder(&url, &page, Some(&page)), || {
        ProxyPrefsGuard::set(&proxy_uri, "", "")
    });
    assert!(established(&opened), "the socket opens through the proxy");
    assert_eq!(
        proxy.tunnels_to(port),
        1,
        "the proxy saw the WebSocket: {:?}",
        proxy.seen()
    );
    assert_eq!(
        handshakes.lock().unwrap().len(),
        1,
        "the server got the handshake"
    );
}

#[test]
fn a_websocket_to_a_no_proxy_host_does_not_use_the_proxy() {
    let (port, handshakes) = websocket_server(None);
    let proxy = FakeProxy::start(false);
    let state = Arc::new(create_http_state(None));
    let url = format!("ws://127.0.0.1:{port}/");
    let page = format!("http://127.0.0.1:{port}");

    let proxy_uri = proxy.uri();
    let opened = open_socket(&state, socket_builder(&url, &page, Some(&page)), || {
        ProxyPrefsGuard::set(&proxy_uri, "", "127.0.0.1")
    });
    assert!(established(&opened), "the socket opens directly");
    assert_eq!(
        proxy.tunnels_to(port),
        0,
        "a no_proxy host stays off the proxy: {:?}",
        proxy.seen()
    );
    assert_eq!(handshakes.lock().unwrap().len(), 1);
}

#[test]
fn a_websocket_without_a_proxy_opens_as_before() {
    let (port, handshakes) = websocket_server(None);
    let state = Arc::new(create_http_state(None));
    let url = format!("ws://127.0.0.1:{port}/");
    let page = format!("http://127.0.0.1:{port}");

    let opened = open_socket(&state, socket_builder(&url, &page, Some(&page)), || ());
    assert!(established(&opened));
    assert_eq!(handshakes.lock().unwrap().len(), 1);
}

#[test]
fn the_server_sees_sec_gpc_and_no_cookie_of_a_third_party_socket() {
    let (port, handshakes) = websocket_server(None);
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    let url = format!("ws://127.0.0.1:{port}/");
    store_cookie(&state, &format!("http://127.0.0.1:{port}/"), "session=abc");

    let opened = open_socket(
        &state,
        socket_builder(&url, "http://other.test", Some("http://other.test")),
        || (),
    );
    assert!(established(&opened));

    let handshakes = handshakes.lock().unwrap();
    assert_eq!(handshakes.len(), 1);
    assert_eq!(handshakes[0].get("sec-gpc").map(String::as_str), Some("1"));
    assert!(
        !handshakes[0].contains_key("cookie"),
        "a third-party socket carries no cookie: {:?}",
        handshakes[0].keys().collect::<Vec<_>>()
    );
}

#[test]
fn the_server_sees_the_cookie_of_a_first_party_socket() {
    let (port, handshakes) = websocket_server(None);
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    let url = format!("ws://127.0.0.1:{port}/");
    let page = format!("http://127.0.0.1:{port}");
    store_cookie(&state, &format!("http://127.0.0.1:{port}/"), "session=abc");

    let opened = open_socket(&state, socket_builder(&url, &page, Some(&page)), || ());
    assert!(established(&opened));

    let handshakes = handshakes.lock().unwrap();
    assert_eq!(handshakes.len(), 1);
    assert_eq!(handshakes[0].get("sec-gpc").map(String::as_str), Some("1"));
    assert_eq!(
        handshakes[0].get("cookie").map(String::as_str),
        Some("session=abc")
    );
}

#[test]
fn a_set_cookie_of_the_answer_is_stored_for_a_first_party_socket_and_ignored_for_a_third_party_one()
{
    fn cookies_of_server(state: &Arc<HttpState>, port: u16) -> Option<String> {
        let url = ServoUrl::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        state
            .cookie_jar
            .write()
            .cookies_for_url(&url, CookieSource::HTTP)
    }

    // First party: stored.
    let (port, _) = websocket_server(Some("from_socket=1"));
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    let url = format!("ws://127.0.0.1:{port}/");
    let page = format!("http://127.0.0.1:{port}");
    let opened = open_socket(&state, socket_builder(&url, &page, Some(&page)), || ());
    assert!(established(&opened));
    assert_eq!(
        cookies_of_server(&state, port).as_deref(),
        Some("from_socket=1")
    );

    // Third party: ignored.
    let (port, _) = websocket_server(Some("from_socket=1"));
    let state = Arc::new(create_http_state(None));
    mercury_policy(&state);
    let url = format!("ws://127.0.0.1:{port}/");
    let opened = open_socket(
        &state,
        socket_builder(&url, "http://other.test", Some("http://other.test")),
        || (),
    );
    assert!(established(&opened));
    assert_eq!(cookies_of_server(&state, port), None);

    // The policy off: stored, as Servo does.
    let (port, _) = websocket_server(Some("from_socket=1"));
    let state = Arc::new(create_http_state(None));
    let url = format!("ws://127.0.0.1:{port}/");
    let opened = open_socket(
        &state,
        socket_builder(&url, "http://other.test", Some("http://other.test")),
        || (),
    );
    assert!(established(&opened));
    assert_eq!(
        cookies_of_server(&state, port).as_deref(),
        Some("from_socket=1")
    );
}
