// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! View source and images per site in a real engine, through Mercury's own runtime: the real
//! `BrowserDelegate` (so `refuse_embedded_view_source`, the navigation guard, HTTPS-only and the
//! images guard see every load), the real protocol registry, the real cookie jar.
//!
//! None of this has ever been run: every test here is `#[ignore]`d and goes to the hands-on list
//! (`sdd/deferred-hands-on.md`, section A), to be run on the integration build before the menu
//! rows are trusted. They are the pass/fail gate for the two features:
//!
//! 1. `a_source_address_loads_as_a_tabs_own_page_with_the_engines_cookies_and_headers` answers
//!    whether Servo 0.8 loads the registered `view-source:` scheme as a top-level page, whether
//!    the handler's answer renders, and whether the fetch sends the real jar's cookie and the
//!    engine's `User-Agent`;
//! 2. `a_page_cannot_reach_a_source_page` loads a page whose `<iframe src>`, `fetch()`,
//!    `window.open()` and `location.href` all name a `view-source:` address: the server must
//!    never be asked for any of them;
//! 3. `the_csp_of_the_source_page_stops_injected_content` checks that Servo enforces the
//!    page's `Content-Security-Policy` (the exact header `page_headers()` gives) and that
//!    hostile source text does not become markup;
//! 4. `images_off_for_a_site_stop_every_image_request_even_after_a_cross_site_navigation`
//!    counts the image requests that reach a loopback server.
//!
//! Why `#[ignore]`: each test starts Servo with a software rendering context, which still opens
//! a GL context and may hang while the screen is locked, so a lane cannot run it (decision 5 of
//! `lead-decisions.md`). Run each one in its own process, on an unlocked screen, in the GStreamer
//! environment (`eval "$(scripts/macos/gstreamer.sh env)"`), with no proxy variables set:
//!
//! ```text
//! for t in a_source_address_loads_as_a_tabs_own_page_with_the_engines_cookies_and_headers \
//!          a_page_cannot_reach_a_source_page \
//!          the_csp_of_the_source_page_stops_injected_content \
//!          images_off_for_a_site_stop_every_image_request_even_after_a_cross_site_navigation; do
//!   cargo test --release -p talaria-servo --test view_source_engine -- --ignored --exact "$t" --nocapture
//! done
//! ```
//!
//! The runtime is `ServoRuntime` with `RuntimeOptions::single_process`, because a test binary
//! cannot be the content process the application starts from itself. The server is a set of
//! threads on `127.0.0.1:0`; `localhost` reaches it too, and is the second "site" of the
//! cross-site test. Nothing leaves the machine. What a page shows the test is its `document.title`
//! (the web view's title events); nothing here evaluates script in the page from outside.

use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use servo::{RenderingContext, SoftwareRenderingContext};
use talaria_core::{BrowserEvent, LoadState, SiteImages, TabEvent, TabId, WebView};
use talaria_servo::{EventLoopWaker, RuntimeOptions, ServoRuntime, ServoWebView};

/// How long any wait may take before the test gives up instead of hanging.
const WAIT: Duration = Duration::from_secs(90);
/// How long to keep running the engine after a page loaded, so that a request that should not
/// happen has time to.
const SETTLE: Duration = Duration::from_millis(2500);

/// A 1x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f,
    0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0xcc, 0x59, 0xe7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

// ---------------------------------------------------------------------------------------------
// A loopback server that records what it was asked for.
// ---------------------------------------------------------------------------------------------

/// One request as the server saw it.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What the server answers.
struct Answer {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn html(body: &str) -> Self {
        Self {
            headers: vec![(
                "Content-Type".to_owned(),
                "text/html; charset=utf-8".to_owned(),
            )],
            body: body.as_bytes().to_vec(),
        }
    }

    fn with(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    /// Serves `route(path, port)` on a loopback port, one thread per connection.
    fn start(route: impl Fn(&str, u16) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = listener.local_addr().expect("an address").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let route = Arc::new(route);
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let route = Arc::clone(&route);
                let log = Arc::clone(&log);
                std::thread::spawn(move || serve(stream, port, route.as_ref(), &log));
            }
        });
        Self { port, seen }
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn asked_for(&self, prefix: &str) -> Vec<Seen> {
        self.requests()
            .into_iter()
            .filter(|seen| seen.path.starts_with(prefix))
            .collect()
    }
}

fn serve(
    mut stream: TcpStream,
    port: u16,
    route: &(dyn Fn(&str, u16) -> Answer + Send + Sync),
    log: &Mutex<Vec<Seen>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut head = Vec::new();
    let mut buffer = [0_u8; 2048];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => head.extend_from_slice(&buffer[..count]),
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let target = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/")
        .to_owned();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Seen {
            path: target.clone(),
            headers,
        });
    let answer = route(&target, port);
    let mut response = String::from("HTTP/1.1 200 OK\r\n");
    for (name, value) in &answer.headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        answer.body.len()
    ));
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(&answer.body);
}

// ---------------------------------------------------------------------------------------------
// Mercury's own runtime, in this process.
// ---------------------------------------------------------------------------------------------

struct Browser {
    // Declared before `runtime`: the web views go before the engine.
    views: RefCell<Vec<(TabId, ServoWebView)>>,
    events: RefCell<Vec<(TabId, BrowserEvent)>>,
    receiver: Receiver<TabEvent>,
    sender: Sender<TabEvent>,
    context: Rc<dyn RenderingContext>,
    next_tab: Cell<u64>,
    runtime: ServoRuntime,
}

impl Browser {
    fn new() -> Self {
        let context: Rc<dyn RenderingContext> = Rc::new(
            SoftwareRenderingContext::new(PhysicalSize {
                width: 800,
                height: 600,
            })
            .expect("a software rendering context"),
        );
        assert!(context.make_current().is_ok());
        let runtime = ServoRuntime::new(
            EventLoopWaker::new(|| {}),
            RuntimeOptions {
                gpu_canvas: false,
                experimental_web_features: false,
                // Content blocking would answer some loads before the guards under test see them.
                block_ads_and_trackers: false,
                single_process: true,
                ..RuntimeOptions::default()
            },
        );
        let (sender, receiver) = mpsc::channel();
        Self {
            views: RefCell::new(Vec::new()),
            events: RefCell::new(Vec::new()),
            receiver,
            sender,
            context,
            next_tab: Cell::new(1),
            runtime,
        }
    }

    /// Opens a tab on `address` and returns it.
    fn open(&self, address: &str) -> TabId {
        let tab = TabId::new(self.next_tab.get());
        self.next_tab.set(self.next_tab.get() + 1);
        let view = self
            .runtime
            .create_web_view(
                Rc::clone(&self.context),
                address,
                1.0,
                tab,
                self.sender.clone(),
            )
            .expect("a web view");
        self.views.borrow_mut().push((tab, view));
        tab
    }

    /// Navigates a tab, as the address bar does.
    fn navigate(&self, tab: TabId, address: &str) {
        let mut views = self.views.borrow_mut();
        let (_, view) = views
            .iter_mut()
            .find(|(id, _)| *id == tab)
            .expect("the tab exists");
        view.navigate(address).expect("a navigation");
    }

    /// One turn of the loop the application runs.
    fn pump(&self) {
        self.runtime.spin_event_loop();
        for (_, view) in self.views.borrow().iter() {
            view.refresh_page_feedback();
            view.paint_if_available();
        }
        while let Ok(event) = self.receiver.try_recv() {
            let tab = event.tab_id();
            self.events.borrow_mut().push((tab, event.into_event()));
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    fn wait(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
            self.pump();
        }
    }

    /// Runs the engine for `SETTLE`, so that a request that should not happen can.
    fn settle(&self) {
        let start = Instant::now();
        while start.elapsed() < SETTLE {
            self.pump();
        }
    }

    fn title(&self, tab: TabId) -> Option<String> {
        self.events
            .borrow()
            .iter()
            .rev()
            .find_map(|(id, event)| match event {
                BrowserEvent::PageTitleChanged { title: Some(title) } if *id == tab => {
                    Some(title.clone())
                }
                _ => None,
            })
    }

    fn loads(&self, tab: TabId) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|(id, event)| {
                *id == tab
                    && matches!(
                        event,
                        BrowserEvent::LoadStateChanged {
                            state: LoadState::Loaded
                        }
                    )
            })
            .count()
    }

    fn wait_title(&self, tab: TabId, wanted: impl Fn(&str) -> bool, what: &str) -> String {
        self.wait(what, |browser| {
            browser.title(tab).is_some_and(|title| wanted(&title))
        });
        self.title(tab).expect("a title")
    }

    fn wait_loaded(&self, tab: TabId, loads: usize, what: &str) {
        self.wait(what, |browser| browser.loads(tab) >= loads);
    }
}

fn page(body: &str) -> Answer {
    Answer::html(&format!("<!doctype html>\n<title>t</title>\n{body}\n"))
}

// ---------------------------------------------------------------------------------------------
// 1. The page loads, with the engine's cookies and headers.
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs a real engine: a GL context that may hang while the screen is locked"]
fn a_source_address_loads_as_a_tabs_own_page_with_the_engines_cookies_and_headers() {
    let server = Server::start(|path, _| match path {
        "/set-cookie" => page("<p>cookie set</p>").with("Set-Cookie", "sid=jar-test; Path=/"),
        _ => page("<p id=\"second\">second line</p>"),
    });
    let port = server.port;
    let browser = Browser::new();

    // An ordinary page sets a cookie in the engine's own jar.
    let first = browser.open(&format!("http://127.0.0.1:{port}/set-cookie"));
    browser.wait_loaded(first, 1, "the page that sets a cookie");

    // The source tab: the handler is reached as a top-level load and its answer renders.
    let source = browser.open(&format!("view-source:http://127.0.0.1:{port}/echo"));
    let title = browser.wait_title(
        source,
        |title| title.starts_with("Source of "),
        "the source page's title",
    );
    assert_eq!(title, "Source of 127.0.0.1/echo");

    // What the re-fetch sent: the jar's cookie, the engine's `User-Agent`, and nothing a page
    // load would not send as a first, top-level request.
    let asked = server.asked_for("/echo");
    assert_eq!(asked.len(), 1, "the page was fetched once: {asked:?}");
    let request = &asked[0];
    assert_eq!(request.header("cookie"), Some("sid=jar-test"));
    assert_eq!(
        request.header("user-agent"),
        Some(browser.runtime.user_agent())
    );
    assert!(request.header("accept-language").is_some());
    assert_eq!(request.header("referer"), None);
    assert!(
        request
            .headers
            .iter()
            .all(|(name, _)| !name.starts_with("sec-fetch-")),
        "{:?}",
        request.headers
    );

    // An address that is not a web page is refused by the handler, without any request.
    let before = server.requests().len();
    let refused = browser.open("view-source:ftp://127.0.0.1/");
    browser.wait_loaded(refused, 1, "the refusal");
    assert_eq!(server.requests().len(), before);
}

// ---------------------------------------------------------------------------------------------
// 2. A page cannot reach the handler.
// ---------------------------------------------------------------------------------------------

const GUARDS_PAGE: &str = r#"
<iframe id="f" src="view-source:http://127.0.0.1:{p}/secret-frame"></iframe>
<script>
var out = [];
window.addEventListener('load', function () {
  fetch('view-source:http://127.0.0.1:{p}/secret-fetch').then(
    function () { out.push('fetch=resolved'); },
    function () { out.push('fetch=rejected'); }
  ).then(function () {
    try {
      var w = window.open('view-source:http://127.0.0.1:{p}/secret-open');
      out.push('open=' + (w ? 'window' : 'null'));
    } catch (e) { out.push('open=threw'); }
    try {
      out.push('frame=' + (document.getElementById('f').contentDocument ? 'document' : 'none'));
    } catch (e) { out.push('frame=blocked'); }
    setTimeout(function () {
      location.href = 'view-source:http://127.0.0.1:{p}/secret-nav';
      setTimeout(function () { document.title = 'RESULT ' + out.join(';'); }, 1500);
    }, 300);
  });
});
</script>
"#;

#[test]
#[ignore = "needs a real engine: a GL context that may hang while the screen is locked"]
fn a_page_cannot_reach_a_source_page() {
    let server = Server::start(|path, port| match path {
        "/guards" => page(&GUARDS_PAGE.replace("{p}", &port.to_string())),
        _ => page("<p>secret</p>"),
    });
    let port = server.port;
    let browser = Browser::new();

    let tab = browser.open(&format!("http://127.0.0.1:{port}/guards"));
    let result = browser.wait_title(
        tab,
        |title| title.starts_with("RESULT "),
        "the page's report",
    );

    // The page stayed where it was (its own script reported), and `fetch()` was refused.
    assert!(result.contains("fetch=rejected"), "{result}");
    // The handler fetches the address it is given: if any of the four had reached it, the server
    // would have been asked for it.
    for secret in [
        "/secret-frame",
        "/secret-fetch",
        "/secret-open",
        "/secret-nav",
    ] {
        assert!(
            server.asked_for(secret).is_empty(),
            "a page reached the source handler through {secret}: {result}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// 3. The CSP of the source page, and hostile source text.
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs a real engine: a GL context that may hang while the screen is locked"]
fn the_csp_of_the_source_page_stops_injected_content() {
    let server = Server::start(|path, port| match path {
        // The source of this page is hostile text; the handler must show it, not run it.
        "/hostile" => Answer::html(&format!(
            "</pre></div><img src=\"http://127.0.0.1:{port}/inj-img\">\
             <script>document.title='INJECTED'</script>"
        )),
        // A page that carries exactly the headers of the source page, with live content that
        // the policy must stop.
        "/csp-page" => {
            let mut answer = Answer::html(&format!(
                "<!doctype html><title>csp</title><img src=\"http://127.0.0.1:{port}/csp-img\">\
                 <script>document.title='SCRIPT RAN'</script><p>text</p>"
            ));
            answer.headers.clear();
            for (name, value) in mercury_view_source::page_headers() {
                answer = answer.with(name, value);
            }
            answer
        }
        _ => page("<p>x</p>"),
    });
    let port = server.port;
    let browser = Browser::new();

    // End to end: hostile source is text on the source page.
    let source = browser.open(&format!("view-source:http://127.0.0.1:{port}/hostile"));
    let title = browser.wait_title(source, |t| t.starts_with("Source of "), "the source page");
    browser.settle();
    assert_eq!(title, "Source of 127.0.0.1/hostile");
    assert_eq!(
        browser.title(source).as_deref(),
        Some("Source of 127.0.0.1/hostile")
    );
    assert!(
        server.asked_for("/inj-img").is_empty(),
        "an image of the source text was requested"
    );

    // The policy on its own: Servo applies the header the page crate produces.
    let tab = browser.open(&format!("http://127.0.0.1:{port}/csp-page"));
    browser.wait_loaded(tab, 1, "the policy test page");
    browser.settle();
    assert!(
        server.asked_for("/csp-img").is_empty(),
        "the policy did not stop an image"
    );
    assert_ne!(
        browser.title(tab).as_deref(),
        Some("SCRIPT RAN"),
        "the policy did not stop an inline script"
    );
}

// ---------------------------------------------------------------------------------------------
// 4. Images off for a site: count the requests.
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs a real engine: a GL context that may hang while the screen is locked"]
fn images_off_for_a_site_stop_every_image_request_even_after_a_cross_site_navigation() {
    let images: String = (0..30)
        .map(|number| format!("<img src=\"/img/{number}.png\" width=\"4\" height=\"4\">"))
        .collect();
    let server = Server::start(move |path, _| {
        if path.starts_with("/img/") {
            return Answer {
                headers: vec![("Content-Type".to_owned(), "image/png".to_owned())],
                body: PNG.to_vec(),
            };
        }
        match path {
            "/images" => page(&format!(
                "<link rel=\"icon\" href=\"/img/icon.png\">{images}"
            )),
            _ => page("<p>start</p>"),
        }
    });
    let port = server.port;
    let browser = Browser::new();
    // `127.0.0.1` is the site with images off; `localhost` reaches the same server and is another
    // site.
    browser
        .runtime
        .site_images()
        .replace(SiteImages::from_hosts(["127.0.0.1".to_owned()]));
    let images_of = |host: &str| {
        server
            .asked_for("/img/")
            .into_iter()
            .filter(|seen| {
                seen.header("host")
                    .is_some_and(|name| name.starts_with(host))
            })
            .count()
    };

    // Control: the same page on the site whose images are on asks for its images.
    let tab = browser.open(&format!("http://localhost:{port}/images"));
    browser.wait_loaded(tab, 1, "the page of the allowed site");
    browser.settle();
    let allowed = images_of("localhost");
    assert!(allowed > 0, "the control page asked for no images at all");

    // The tab goes from the allowed site to the blocked one: an arrival, whose first images
    // (the parser's prefetch) are asked for before `web_view.url()` changes.
    browser.navigate(tab, &format!("http://127.0.0.1:{port}/images"));
    browser.wait_loaded(tab, 2, "the page of the blocked site");
    browser.settle();
    let blocked = images_of("127.0.0.1");
    println!("image requests: allowed site {allowed}, blocked site {blocked} (of 31 in the page)");
    assert_eq!(
        blocked, 0,
        "{blocked} image requests reached the server for a site with images off"
    );

    // And as the first page of a tab.
    let fresh = browser.open(&format!("http://127.0.0.1:{port}/images"));
    browser.wait_loaded(fresh, 1, "the blocked site in a new tab");
    browser.settle();
    assert_eq!(images_of("127.0.0.1"), 0);
}
