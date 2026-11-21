// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The `view-source:` protocol handler, and the guard that keeps pages away from it.
//!
//! `view-source:<address>` is a page of Mercury's own, shown as a tab's top-level document. The
//! handler answers it from `mercury_view_source`: it fetches the page again (with the engine's
//! user agent, the cookies the engine would send, and the proxy the engine uses), highlights the
//! text and answers a self-contained HTML page. It runs on Servo's network runtime, inside the
//! fetch of the main frame, so nothing here may block that runtime or panic through it:
//!
//! - at most [`MAX_CONCURRENT_FETCHES`] pages are fetched and rendered at once;
//! - the fetch is awaited under `catch_unwind`, so a panic gives the error page for
//!   `FetchError::Internal` and never reaches the network thread;
//! - highlighting is CPU work in C (tree-sitter): it runs on a blocking thread, never on a
//!   runtime worker, and under `catch_unwind` too;
//! - a request that is not a `GET`, or whose address is not `view-source:` and an `http(s)`
//!   address, is answered with a small plain-text page and no fetch (`check_request`);
//! - dropping the answer (the tab navigated away) drops the fetch, which closes its connection.
//!
//! A page never reaches the handler. The guards, from the outside in: `navigation_verdict`
//! refuses a `view-source:` navigation that a web page asks for (`talaria-core`);
//! [`refuse_embedded_view_source`] answers `403` to any `view-source:` load that is not the
//! tab's own main frame (a frame, an image, a script, a worker, `fetch()` with a `Request`
//! built by the page); the handler is not fetchable and not secure (`fetch()` and
//! XMLHttpRequest of the scheme fail, and a secure context is not granted); and the page's
//! origin is opaque. A hole is closed in a guard, never in the handler.
//!
//! Whether Servo 0.8 loads a registered custom scheme as a top-level page has not been run on
//! this machine (the screen was locked); `tests/view_source_engine.rs` is the test for it and is
//! on the hands-on list.
//!
//! The first half of this file holds no engine type, so that what the handler answers is tested
//! without an engine; the second half is the glue to Servo's network stack.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{Either, select};
use http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use mercury_view_source::{
    CookieLookup, FetchError, FetchOptions, ProxyConfig, SourceDocument, ViewSourcePage,
    ViewSourceUrl, fetch_source, page_headers, render_error_page, render_page,
};
use tokio::sync::{Semaphore, oneshot};

/// The most pages fetched and rendered at once; the others wait for a place.
const MAX_CONCURRENT_FETCHES: usize = 4;
/// How long connecting to the server may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a fetch may take in all.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long past its own total timeout a fetch may still run before the handler gives up on it:
/// the fetch keeps its own clock, this one is for a fetch that does not.
const OVERRUN: Duration = Duration::from_secs(5);

/// What the handler answers, without a single engine type.
#[derive(Debug)]
struct Reply {
    status: u16,
    reason: &'static str,
    headers: HeaderMap,
    body: Vec<u8>,
}

/// A small plain-text answer. It repeats nothing of the request: no address, no method.
fn plain_reply(status: u16, reason: &'static str, text: &str) -> Reply {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Reply {
        status,
        reason,
        headers,
        body: text.as_bytes().to_vec(),
    }
}

/// The answer to a method other than `GET`: a source page can only be read.
fn bad_method_reply() -> Reply {
    let mut reply = plain_reply(
        405,
        "Method Not Allowed",
        "Method not allowed: a source page can only be read.",
    );
    reply.headers.insert(ALLOW, HeaderValue::from_static("GET"));
    reply
}

/// The answer to an address that is not `view-source:` and an `http(s)` address.
fn bad_address_reply() -> Reply {
    plain_reply(
        400,
        "Bad Request",
        "Bad request: only the address of an http or https page has a source.",
    )
}

/// The answer when not even the error page could be made. Fixed text.
fn fallback_reply() -> Reply {
    plain_reply(
        500,
        "Internal Server Error",
        "The source could not be shown.",
    )
}

/// A generated page as an answer: status 200, with `page_headers()`. A header the page crate
/// names badly is left out and logged, never panicked on.
fn page_reply(page: ViewSourcePage) -> Reply {
    let mut headers = HeaderMap::new();
    for (name, value) in page_headers() {
        match (HeaderName::try_from(name), HeaderValue::try_from(value)) {
            (Ok(name), Ok(value)) => {
                headers.insert(name, value);
            }
            _ => log::error!("view-source: a page header could not be set: {name}"),
        }
    }
    Reply {
        status: 200,
        reason: "OK",
        headers,
        body: page.html.into_bytes(),
    }
}

/// What a request asks for: the address to show the source of, or the answer to refuse it with.
/// The method is judged first, so a `POST` to a bad address is a `405`.
fn check_request(method: &Method, address: &str) -> Result<ViewSourceUrl, Box<Reply>> {
    if *method != Method::GET {
        return Err(Box::new(bad_method_reply()));
    }
    ViewSourceUrl::parse(address).map_err(|_| Box::new(bad_address_reply()))
}

/// What a fetch needs from the engine, with this handler's limits.
fn fetch_options(user_agent: String, cookies: CookieLookup, proxy: ProxyConfig) -> FetchOptions {
    FetchOptions {
        user_agent,
        cookies: Some(cookies),
        proxy,
        connect_timeout: CONNECT_TIMEOUT,
        total_timeout: TOTAL_TIMEOUT,
    }
}

/// The fetch of `target`, for the handler: the crate's `fetch_source` under a clock of the
/// handler's own, a little longer than the fetch's.
async fn fetch_future(
    target: ViewSourceUrl,
    options: FetchOptions,
) -> Result<SourceDocument, FetchError> {
    let limit = options.total_timeout + OVERRUN;
    match tokio::time::timeout(limit, fetch_source(&target, &options)).await {
        Ok(outcome) => outcome,
        Err(_) => Err(FetchError::Timeout),
    }
}

/// Runs a fetch; a panic in it is `FetchError::Internal`, not a crash of the network thread.
async fn guarded<F>(fetch: F) -> Result<SourceDocument, FetchError>
where
    F: Future<Output = Result<SourceDocument, FetchError>>,
{
    match AssertUnwindSafe(fetch).catch_unwind().await {
        Ok(outcome) => outcome,
        Err(_) => {
            log::error!("view-source: the fetch panicked");
            Err(FetchError::Internal)
        }
    }
}

/// The page for what the fetch gave. Blocking: highlighting is CPU work, so this runs on a thread
/// that may block. A panic while making the page gives the error page for `Internal`, and a panic
/// even there gives the fixed fallback.
fn render_reply(target: &ViewSourceUrl, outcome: Result<SourceDocument, FetchError>) -> Reply {
    let page = std::panic::catch_unwind(AssertUnwindSafe(|| match &outcome {
        Ok(document) => render_page(document, target),
        Err(error) => render_error_page(target, error),
    }))
    .or_else(|_| {
        log::error!("view-source: the page could not be generated");
        std::panic::catch_unwind(AssertUnwindSafe(|| {
            render_error_page(target, &FetchError::Internal)
        }))
    });
    page.map_or_else(|_| fallback_reply(), page_reply)
}

/// The whole answer for `target`: waits for a place among `permits`, runs `fetch`, and makes the
/// page. The place is held until the page is made and is given back when this future is dropped.
///
/// Inside a tokio runtime (Servo's, where the handler runs) the fetch is awaited here and the page
/// is made on a blocking task. Without one (an executor that is not tokio's) the fetch runs on a
/// small thread of its own with a runtime of its own, so a fetch cannot panic for want of one.
async fn answer<F>(target: ViewSourceUrl, fetch: F, permits: Arc<Semaphore>) -> Reply
where
    F: Future<Output = Result<SourceDocument, FetchError>> + Send + 'static,
{
    let Ok(_place) = permits.acquire_owned().await else {
        return render_reply(&target, Err(FetchError::Internal));
    };
    if tokio::runtime::Handle::try_current().is_err() {
        return answer_on_own_thread(target, fetch).await;
    }
    let outcome = guarded(fetch).await;
    let made = tokio::task::spawn_blocking(move || render_reply(&target, outcome));
    made.await.unwrap_or_else(|_| fallback_reply())
}

/// `answer` for a caller without a tokio runtime: the fetch and the page are made on a thread of
/// their own, which stops early if the answer is not wanted any more.
async fn answer_on_own_thread<F>(target: ViewSourceUrl, fetch: F) -> Reply
where
    F: Future<Output = Result<SourceDocument, FetchError>> + Send + 'static,
{
    let (mut sender, receiver) = oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("view-source".to_owned())
        .spawn(move || {
            let outcome = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(async {
                    let work = Box::pin(guarded(fetch));
                    let unwanted = Box::pin(sender.closed());
                    match select(work, unwanted).await {
                        Either::Left((outcome, _)) => Some(outcome),
                        Either::Right(_) => None,
                    }
                }),
                Err(_) => Some(Err(FetchError::Internal)),
            };
            if let Some(outcome) = outcome {
                // The receiver is gone if the answer is not wanted any more.
                let _ = sender.send(render_reply(&target, outcome));
            }
        });
    if spawned.is_err() {
        log::error!("view-source: no thread for the fetch");
        return fallback_reply();
    }
    receiver.await.unwrap_or_else(|_| fallback_reply())
}

/// Whether a load of `scheme` is a `view-source:` load that must be refused: any that is not a
/// web view's own main frame. `has_web_view` is false for loads that name no web view (workers).
fn is_embedded_view_source(scheme: &str, is_for_main_frame: bool, has_web_view: bool) -> bool {
    scheme == mercury_view_source::SCHEME && (!is_for_main_frame || !has_web_view)
}

// ---------------------------------------------------------------------------------------------
// The glue to Servo's network stack: nothing below is tested without an engine.
// ---------------------------------------------------------------------------------------------

use std::future::ready;
use std::pin::Pin;

use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, ProtocolHandler, ProtocolRegistry, Request,
    ResourceFetchTiming, Response, ResponseBody,
};
use servo::{CookieSource, ServoUrl, WebResourceLoad};

/// What the handler needs that the engine's request does not carry.
#[derive(Clone, Debug)]
pub struct ViewSourceConfig {
    /// Servo's three proxy preferences, as they are: the fetch goes through the proxy the page
    /// went through, or fails; it never connects directly instead.
    pub proxy: ProxyConfig,
}

/// Registers the `view-source` scheme. The handler is neither fetchable nor secure.
pub fn register(registry: &mut ProtocolRegistry, config: ViewSourceConfig) {
    let handler = ViewSourceHandler {
        proxy: config.proxy,
        permits: Arc::new(Semaphore::new(MAX_CONCURRENT_FETCHES)),
    };
    if let Err(error) = registry.register(mercury_view_source::SCHEME, handler) {
        log::error!(
            "could not register {}: {error:?}",
            mercury_view_source::SCHEME
        );
    }
}

struct ViewSourceHandler {
    proxy: ProxyConfig,
    permits: Arc<Semaphore>,
}

impl ProtocolHandler for ViewSourceHandler {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let timing = ResourceFetchTiming::new(request.timing_type());
        let target = match check_request(&request.method, url.as_str()) {
            Ok(target) => target,
            Err(reply) => return Box::pin(ready(to_response(url, timing, *reply))),
        };
        // The cookies the engine would send for each address of the fetch, from the engine's own
        // jar. The future outlives `context`, so it holds a clone of the engine's state.
        let state = Arc::clone(&context.state);
        let cookies: CookieLookup = Arc::new(move |address: &url::Url| {
            state
                .cookie_jar
                .write()
                .cookies_for_url(&ServoUrl::from(address.clone()), CookieSource::HTTP)
        });
        let options = fetch_options(context.user_agent.clone(), cookies, self.proxy.clone());
        let permits = Arc::clone(&self.permits);
        Box::pin(async move {
            let fetch = fetch_future(target.clone(), options);
            let reply = answer(target, fetch, permits).await;
            to_response(url, timing, reply)
        })
    }

    /// `fetch()` and XMLHttpRequest of the scheme fail: a page cannot read a source.
    fn is_fetchable(&self) -> bool {
        false
    }

    /// A source page is not a secure context and gives no secure context to anything.
    fn is_secure(&self) -> bool {
        false
    }
}

/// The engine's response for a reply. The statuses here are those of the constructors above, all
/// inside the range `HttpStatus::new_raw` accepts.
fn to_response(url: ServoUrl, timing: ResourceFetchTiming, reply: Reply) -> Response {
    let mut response = Response::new(url, timing);
    response.status = HttpStatus::new_raw(
        reply.status.clamp(100, 599),
        reply.reason.as_bytes().to_vec(),
    );
    response.headers = reply.headers;
    *response.body.lock() = ResponseBody::Done(reply.body);
    response
}

/// Refuses `load` if it asks for a `view-source:` address as anything but a tab's own main frame
/// (a frame's `src`, an image, a script, `fetch()`): answered `403`, the same empty answer
/// `openurl` gives a refused local file, so that a frame gets an empty document and its parent
/// still finishes loading. Otherwise gives it back. For a load that names a web view.
pub(crate) fn refuse_embedded_view_source(load: WebResourceLoad) -> Option<WebResourceLoad> {
    refuse_view_source_load(load, true)
}

/// `refuse_embedded_view_source` for a load that names no web view (a worker's): every
/// `view-source:` load is refused, a main-frame one too, since no tab shows it.
pub(crate) fn refuse_view_source_without_web_view(
    load: WebResourceLoad,
) -> Option<WebResourceLoad> {
    refuse_view_source_load(load, false)
}

fn refuse_view_source_load(load: WebResourceLoad, has_web_view: bool) -> Option<WebResourceLoad> {
    let request = load.request();
    if !is_embedded_view_source(
        request.url.scheme(),
        request.is_for_main_frame,
        has_web_view,
    ) {
        return Some(load);
    }
    // Not the address: it may carry what the page wanted to hide.
    log::info!(
        "refused a view-source: load that is not a tab's own page (for a main frame: {}, web view: {})",
        request.is_for_main_frame,
        has_web_view
    );
    let url = request.url.as_url().clone();
    crate::openurl::answer_forbidden(load, url);
    None
}

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    use mercury_view_source::SourceKind;
    use url::Url;

    use super::*;

    fn target(text: &str) -> ViewSourceUrl {
        ViewSourceUrl::parse(text).expect("the test address is valid")
    }

    fn address() -> ViewSourceUrl {
        target("view-source:https://example.com/index.html")
    }

    fn document(text: &str) -> SourceDocument {
        SourceDocument {
            final_url: Url::parse("https://example.com/index.html").expect("a URL"),
            status: 200,
            content_type: Some("text/plain".to_owned()),
            kind: SourceKind::Plain,
            body: mercury_view_source::SourceBody::Text {
                text: text.to_owned(),
                bytes: text.len() as u64,
                truncated: false,
                charset: "UTF-8",
            },
        }
    }

    fn body(reply: &Reply) -> String {
        String::from_utf8_lossy(&reply.body).into_owned()
    }

    fn header<'a>(reply: &'a Reply, name: &str) -> Option<&'a str> {
        reply
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    /// A small runtime with timers and a blocking pool, as much of Servo's network runtime as
    /// the handler uses.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    }

    /// A minimal executor that is not tokio's: polls on this thread, parking while pending.
    fn block_on_without_tokio<T>(future: impl Future<Output = T>) -> T {
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    // ---- what is answered for a request ----

    #[test]
    fn a_get_for_the_source_of_a_web_page_is_fetched() {
        for text in [
            "view-source:https://example.com/",
            "view-source:http://127.0.0.1:8000/a?b=1",
            "VIEW-SOURCE:https://example.com/#fragment",
        ] {
            let Ok(found) = check_request(&Method::GET, text) else {
                panic!("{text} was refused");
            };
            assert!(found.target().scheme().starts_with("http"), "{text}");
        }
    }

    #[test]
    fn every_method_but_get_is_a_405_that_fetches_nothing() {
        for method in [
            Method::POST,
            Method::HEAD,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
            Method::PATCH,
        ] {
            let Err(reply) = check_request(&method, "view-source:https://example.com/") else {
                panic!("{method} was accepted");
            };
            assert_eq!((reply.status, reply.reason), (405, "Method Not Allowed"));
            assert_eq!(header(&reply, "allow"), Some("GET"));
            assert_eq!(
                header(&reply, "content-type"),
                Some("text/plain; charset=utf-8")
            );
        }
    }

    #[test]
    fn the_method_is_judged_before_the_address() {
        let Err(reply) = check_request(&Method::POST, "view-source:file:///etc/hosts") else {
            panic!("a POST was accepted");
        };
        assert_eq!(reply.status, 405);
    }

    #[test]
    fn an_address_that_is_not_the_source_of_a_web_page_is_a_400() {
        for text in [
            "",
            "view-source:",
            "view-source:file:///etc/hosts",
            "view-source:javascript:alert(1)",
            "view-source:data:text/html,x",
            "view-source:view-source:https://example.com/",
            "view-source:ftp://example.com/",
            "view-source:/relative",
            "https://example.com/",
            "about:blank",
        ] {
            let Err(reply) = check_request(&Method::GET, text) else {
                panic!("{text:?} was accepted");
            };
            assert_eq!(
                (reply.status, reply.reason),
                (400, "Bad Request"),
                "{text:?}"
            );
            assert!(reply.headers.get("allow").is_none(), "{text:?}");
        }
    }

    #[test]
    fn a_plain_answer_repeats_nothing_of_the_request() {
        for reply in [bad_method_reply(), bad_address_reply(), fallback_reply()] {
            let text = body(&reply);
            assert!(!text.is_empty());
            assert!(!text.contains("view-source"), "{text}");
            assert!(
                !text.contains("http://") && !text.contains("https://"),
                "{text}"
            );
            assert_eq!(header(&reply, "cache-control"), Some("no-store"));
            assert_eq!(header(&reply, "x-content-type-options"), Some("nosniff"));
            assert_eq!(
                header(&reply, "content-type"),
                Some("text/plain; charset=utf-8")
            );
        }
        assert_eq!(fallback_reply().status, 500);
    }

    // ---- the headers and what the engine is given ----

    #[test]
    fn a_page_is_answered_200_with_the_page_headers_and_nothing_else() {
        let reply = page_reply(ViewSourcePage {
            html: "<!doctype html><title>t</title>".to_owned(),
        });
        assert_eq!((reply.status, reply.reason), (200, "OK"));
        assert_eq!(reply.headers.len(), page_headers().len());
        for (name, value) in page_headers() {
            assert_eq!(header(&reply, name), Some(value), "{name}");
        }
        assert_eq!(body(&reply), "<!doctype html><title>t</title>");
        // the page is never cached and never sniffed
        assert_eq!(header(&reply, "cache-control"), Some("no-store"));
        assert_eq!(header(&reply, "x-content-type-options"), Some("nosniff"));
    }

    #[test]
    fn a_fetch_gets_the_engines_user_agent_cookies_and_proxy_and_the_limits_of_the_handler() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let cookies: CookieLookup = Arc::new(move |url: &Url| {
            log.lock().expect("a lock").push(url.as_str().to_owned());
            Some("a=b".to_owned())
        });
        let proxy = ProxyConfig {
            http: "http://proxy.test:3128".to_owned(),
            https: "http://proxy.test:3129".to_owned(),
            no_proxy: "localhost,.internal".to_owned(),
        };
        let options = fetch_options("Mercury/1".to_owned(), cookies, proxy.clone());
        assert_eq!(options.user_agent, "Mercury/1");
        assert_eq!(options.proxy, proxy);
        assert_eq!(options.connect_timeout, Duration::from_secs(10));
        assert_eq!(options.total_timeout, Duration::from_secs(30));
        let lookup = options.cookies.expect("cookies are asked for");
        let hop = Url::parse("https://example.com/next").expect("a URL");
        assert_eq!(lookup(&hop).as_deref(), Some("a=b"));
        assert_eq!(*asked.lock().expect("a lock"), ["https://example.com/next"]);
    }

    // ---- making the page ----

    #[test]
    fn a_fetched_document_becomes_a_page() {
        let reply = render_reply(&address(), Ok(document("hello world")));
        assert_eq!(reply.status, 200);
        assert_eq!(
            header(&reply, "content-type"),
            Some("text/html; charset=utf-8")
        );
        assert!(body(&reply).contains("hello world"));
    }

    #[test]
    fn every_failed_fetch_becomes_a_page_that_says_so() {
        for error in [
            FetchError::Unreachable,
            FetchError::Timeout,
            FetchError::Tls,
            FetchError::TooManyRedirects,
            FetchError::BadRedirect,
            FetchError::Proxy,
            FetchError::Internal,
        ] {
            let reply = render_reply(&address(), Err(error));
            assert_eq!(reply.status, 200, "{error:?}");
            assert_eq!(
                header(&reply, "content-type"),
                Some("text/html; charset=utf-8")
            );
            // `summary` has no character that is escaped
            assert!(body(&reply).contains(error.summary()), "{error:?}");
        }
    }

    // ---- the whole answer ----

    fn answered(
        fetch: impl Future<Output = Result<SourceDocument, FetchError>> + Send + 'static,
    ) -> Reply {
        runtime().block_on(answer(address(), fetch, Arc::new(Semaphore::new(4))))
    }

    #[test]
    fn a_fetched_page_is_answered_inside_a_runtime() {
        let reply = answered(async { Ok(document("fetched text")) });
        assert_eq!(reply.status, 200);
        assert!(body(&reply).contains("fetched text"));
    }

    #[test]
    fn what_the_handler_gives_the_network_stack_can_cross_threads() {
        // Servo awaits the handler's future on its runtime's worker threads. This does not
        // compile if the fetch (the crate's `fetch_source`) holds something that is not `Send`.
        fn is_send<T: Send>(_: &T) {}
        let options = fetch_options(
            "Mercury/1".to_owned(),
            Arc::new(|_: &Url| None),
            ProxyConfig::default(),
        );
        let fetch = fetch_future(address(), options);
        is_send(&fetch);
        let whole = answer(address(), fetch, Arc::new(Semaphore::new(1)));
        is_send(&whole);
    }

    #[test]
    fn a_failed_fetch_is_answered_with_the_page_for_the_error() {
        let reply = answered(async { Err(FetchError::Timeout) });
        assert!(body(&reply).contains(FetchError::Timeout.summary()));
    }

    #[test]
    fn a_panic_in_the_fetch_is_the_internal_error_page_and_goes_no_further() {
        let reply = answered(async {
            if true {
                panic!("the fetch fell over");
            }
            Err(FetchError::Unreachable)
        });
        assert_eq!(reply.status, 200);
        let text = body(&reply);
        assert!(text.contains(FetchError::Internal.summary()), "{text}");
        assert!(
            !text.contains("fell over"),
            "a panic message reached the page"
        );
    }

    #[test]
    fn without_a_tokio_runtime_the_fetch_runs_on_a_thread_of_its_own() {
        let in_a_runtime = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&in_a_runtime);
        let fetch = async move {
            // The thread has a runtime of its own, current for the fetch.
            if tokio::runtime::Handle::try_current().is_ok() {
                seen.fetch_add(1, Ordering::SeqCst);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok(document("fetched on a thread"))
        };
        let reply = block_on_without_tokio(answer(address(), fetch, Arc::new(Semaphore::new(1))));
        assert_eq!(in_a_runtime.load(Ordering::SeqCst), 1);
        assert_eq!(reply.status, 200);
        assert!(body(&reply).contains("fetched on a thread"));
    }

    #[test]
    fn a_panic_in_the_fetch_on_that_thread_is_the_internal_error_page_too() {
        let fetch = async {
            if true {
                panic!("the fetch fell over");
            }
            Err(FetchError::Unreachable)
        };
        let reply = block_on_without_tokio(answer(address(), fetch, Arc::new(Semaphore::new(1))));
        assert!(body(&reply).contains(FetchError::Internal.summary()));
    }

    #[test]
    fn at_most_the_given_number_of_fetches_run_at_once() {
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let permits = Arc::new(Semaphore::new(2));
        let answers = (0..8).map(|_| {
            let (running, most) = (Arc::clone(&running), Arc::clone(&most));
            answer(
                address(),
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    Ok(document("x"))
                },
                Arc::clone(&permits),
            )
        });
        let replies = runtime().block_on(futures_util::future::join_all(answers));
        assert_eq!(replies.len(), 8);
        assert!(replies.iter().all(|reply| reply.status == 200));
        assert_eq!(most.load(Ordering::SeqCst), 2);
        assert_eq!(permits.available_permits(), 2);
    }

    #[test]
    fn dropping_the_answer_gives_the_place_back_and_stops_the_fetch() {
        let permits = Arc::new(Semaphore::new(1));
        let finished = Arc::new(AtomicUsize::new(0));
        let flag = Arc::clone(&finished);
        runtime().block_on(async {
            let pending = answer(
                address(),
                async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    flag.fetch_add(1, Ordering::SeqCst);
                    Ok(document("never"))
                },
                Arc::clone(&permits),
            );
            // Polled once: the place is taken and the fetch waits.
            assert!(pending.now_or_never().is_none());
        });
        assert_eq!(permits.available_permits(), 1);
        assert_eq!(finished.load(Ordering::SeqCst), 0);
    }

    // ---- the guard ----

    #[test]
    fn only_a_tabs_own_main_frame_may_load_a_source_page() {
        // (scheme, is_for_main_frame, has_web_view, refused)
        for (scheme, main_frame, web_view, refused) in [
            ("view-source", true, true, false),
            ("view-source", false, true, true),
            ("view-source", true, false, true),
            ("view-source", false, false, true),
            ("https", false, true, false),
            ("https", true, true, false),
            ("https", false, false, false),
            ("file", false, true, false),
            ("data", false, true, false),
            ("view-source-x", false, true, false),
        ] {
            assert_eq!(
                is_embedded_view_source(scheme, main_frame, web_view),
                refused,
                "{scheme} main frame: {main_frame} web view: {web_view}"
            );
        }
    }
}
