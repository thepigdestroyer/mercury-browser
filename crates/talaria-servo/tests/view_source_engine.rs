// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The `view-source:` protocol handler in a real engine: Servo with the handler registered
//! loads `view-source:http://127.0.0.1:<port>/` as a web view's top-level page.
//!
//! This answers the question the brief leaves open, whether Servo 0.8 loads a registered custom
//! scheme as a top-level navigation. It has never been run: every test here is `#[ignore]`d
//! because it needs the GL stack (Servo's software rendering context still opens a GL context,
//! which may hang while the screen is locked), and goes to the hands-on list. Run it with
//! `cargo test --release -p talaria-servo --test view_source_engine -- --ignored`.
//!
//! The headless pattern is that of `third_party/servo/tests/common/mod.rs`: a
//! `SoftwareRenderingContext`, a `ServoBuilder` whose registry holds the handler, a web view,
//! and `evaluate_javascript` to read the page. The server is a thread on `127.0.0.1:0` that
//! answers every request with one fixed page; nothing leaves the machine. The proxy preferences
//! are empty, as in Servo's own tests, so the loopback fetch is direct.
//!
//! What it reads from the page is what `mercury_view_source::render_page` makes of it: the
//! `<title>` is `Source of <host><path>` and every line of the source is a `div.l`.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use mercury_view_source::ProxyConfig;
use servo::protocol_handler::ProtocolRegistry;
use servo::{
    JSValue, JavaScriptEvaluationError, LoadStatus, Preferences, RenderingContext, Servo,
    ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
};
use talaria_servo::{EventLoopWaker, ViewSourceConfig, register_view_source};

/// How long any wait may take before the test gives up instead of hanging.
const WAIT: Duration = Duration::from_secs(60);

/// The page the server answers with; its first line is `<!doctype html>`.
const SOURCE: &str =
    "<!doctype html>\n<title>Served by the test</title>\n<p id=\"second\">second line</p>\n";

/// Serves `body` as `text/html` to every request on a loopback port; returns the port.
fn serve(body: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("an address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // The request is not looked at: every path gets the same page.
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

/// Paints when the engine has a new frame, as Servo's own tests do.
struct Delegate;

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, web_view: WebView) {
        web_view.paint();
    }
}

/// Servo with the `view-source:` handler registered, on a software rendering context.
struct Engine {
    servo: Servo,
    rendering_context: Rc<dyn RenderingContext>,
}

impl Engine {
    fn new() -> Self {
        let rendering_context = Rc::new(
            SoftwareRenderingContext::new(PhysicalSize {
                width: 500,
                height: 500,
            })
            .expect("a software rendering context"),
        );
        assert!(rendering_context.make_current().is_ok());

        let mut registry = ProtocolRegistry::default();
        register_view_source(
            &mut registry,
            ViewSourceConfig {
                proxy: ProxyConfig::default(),
            },
        );
        // Servo reads the proxy variables of the environment; the fetches here are on loopback.
        let preferences = Preferences {
            network_http_proxy_uri: String::new(),
            network_https_proxy_uri: String::new(),
            network_http_no_proxy: String::new(),
            ..Preferences::default()
        };

        let servo = ServoBuilder::default()
            .preferences(preferences)
            .event_loop_waker(Box::new(EventLoopWaker::new(|| {})))
            .protocol_registry(registry)
            .build();
        Self {
            servo,
            rendering_context,
        }
    }

    /// Spins Servo's event loop until `done` or until `WAIT` has passed; whether `done`.
    fn spin_until(&self, done: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while !done() {
            if start.elapsed() > WAIT {
                return false;
            }
            self.servo.spin_event_loop();
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// A web view that loads `address`, once it has finished loading.
    fn open(&self, address: &str) -> WebView {
        let web_view = WebViewBuilder::new(&self.servo, Rc::clone(&self.rendering_context))
            .delegate(Rc::new(Delegate))
            .url(url::Url::parse(address).expect("a URL"))
            .build();
        assert!(
            self.spin_until(|| matches!(web_view.load_status(), LoadStatus::Complete)),
            "{address} did not finish loading in {WAIT:?}"
        );
        web_view
    }

    fn evaluate(
        &self,
        web_view: &WebView,
        script: &str,
    ) -> Result<JSValue, JavaScriptEvaluationError> {
        let answer = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);
        web_view.evaluate_javascript(script, move |result| *slot.borrow_mut() = Some(result));
        assert!(
            self.spin_until(|| answer.borrow().is_some()),
            "{script} was not answered in {WAIT:?}"
        );
        answer.borrow_mut().take().expect("an answer")
    }
}

fn text(value: Result<JSValue, JavaScriptEvaluationError>) -> String {
    match value {
        Ok(JSValue::String(text)) => text,
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
#[ignore = "needs the GL stack; may hang while the screen is locked"]
fn a_source_address_loads_as_the_top_level_page_and_shows_the_source() {
    let port = serve(SOURCE);
    let engine = Engine::new();
    let web_view = engine.open(&format!("view-source:http://127.0.0.1:{port}/"));

    // The document is Mercury's page, not the served one.
    assert_eq!(
        text(engine.evaluate(&web_view, "document.title")),
        "Source of 127.0.0.1/"
    );
    // The first line of the source is shown as text, not parsed: the served page has a `<title>`
    // of its own, and this page does not.
    assert_eq!(
        text(engine.evaluate(&web_view, "document.querySelector('.l').textContent")).trim(),
        "<!doctype html>"
    );
    // Every line of the source is a line of the page (a last, empty line may be one more).
    assert_eq!(
        text(engine.evaluate(
            &web_view,
            &format!(
                "(document.querySelectorAll('.l').length >= {}).toString()",
                SOURCE.lines().count()
            )
        )),
        "true"
    );
    // No script of the served page ran, and the page has none of its own.
    assert_eq!(
        text(engine.evaluate(&web_view, "String(document.getElementById('second'))")),
        "null"
    );
    assert_eq!(
        text(engine.evaluate(&web_view, "document.scripts.length.toString()")),
        "0"
    );
}

#[test]
#[ignore = "needs the GL stack; may hang while the screen is locked"]
fn an_address_that_is_not_a_web_page_loads_a_plain_refusal() {
    let engine = Engine::new();
    let web_view = engine.open("view-source:ftp://example.com/");

    // Servo shows a plain-text answer as a document of its own; the text is the handler's.
    let shown = text(engine.evaluate(&web_view, "document.body.textContent"));
    assert!(
        shown.contains("only the address of an http or https page has a source"),
        "{shown:?}"
    );
}
