// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use boa_engine::{Context, Source};
use mercury_site_security::{
    CERTIFICATE_EXCEPTION_ACCEPTED, CERTIFICATE_EXCEPTION_SCHEME, ERROR_PAGE_PROBE_SCRIPT,
    ErrorPage, ExceptionRefusal, certificate_error_page, certificate_refusal_line, crash_page,
    network_error_page, parse_error_page_marker,
};

const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// What the engine does with a template (Servo 0.8, `servoparser/mod.rs`).
fn fill(template: &str, reason: &str) -> String {
    template
        .replace("${reason}", reason)
        .replace("${bytes}", "QUJD")
        .replace("${secret}", "424242")
        .replace("${details}", reason)
}

#[test]
fn every_page_holds_its_reason_after_a_plaintext_element_at_the_very_end() {
    for (template, placeholder) in [
        (certificate_error_page(SECRET, false), "${reason}"),
        (network_error_page(SECRET), "${reason}"),
        (crash_page(), "${details}"),
    ] {
        let tail = format!("<plaintext hidden>{placeholder}");
        assert!(template.ends_with(&tail), "{template}");
        assert_eq!(template.matches(placeholder).count(), 1);
        // Hostile text stays text: nothing after <plaintext> is markup.
        let filled = fill(&template, "</plaintext><script>alert(1)</script>");
        let held = filled.rsplit_once("<plaintext hidden>").expect("held").1;
        assert_eq!(held, "</plaintext><script>alert(1)</script>");
    }
}

#[test]
fn pages_carry_a_marker_with_the_secret() {
    let certificate = certificate_error_page(SECRET, false);
    assert!(certificate.contains(&format!(
        "<meta name=\"mercury-error-page\" content=\"certificate {SECRET}\">"
    )));
    let network = network_error_page(SECRET);
    assert!(network.contains(&format!(
        "<meta name=\"mercury-error-page\" content=\"network {SECRET}\">"
    )));
    assert!(!crash_page().contains("mercury-error-page"));
}

#[test]
fn a_secret_cannot_break_out_of_its_attribute() {
    let page = network_error_page("ab\"><script>x</script>cd");
    assert!(page.contains("content=\"network abscriptxscriptcd\""));
}

#[test]
fn the_certificate_page_sends_the_certificate_to_the_exception_scheme() {
    let page = fill(&certificate_error_page(SECRET, false), "bad");
    assert!(page.contains("<meta name=\"mercury-certificate\" content=\"QUJD\">"));
    assert!(page.contains(&format!("fetch('{CERTIFICATE_EXCEPTION_SCHEME}:allow?'")));
    assert!(page.contains("if (false) { proceed.click(); }"));
    let automatic = certificate_error_page(SECRET, true);
    assert!(automatic.contains("if (true) { proceed.click(); }"));
    // Servo's own secret is never used.
    assert!(!certificate_error_page(SECRET, false).contains("${secret}"));
}

#[test]
fn markers_are_recognised_only_with_the_secret() {
    assert_eq!(
        parse_error_page_marker(&format!("network {SECRET}"), SECRET),
        Some(ErrorPage::Network)
    );
    assert_eq!(
        parse_error_page_marker(&format!("certificate {SECRET}"), SECRET),
        Some(ErrorPage::Certificate)
    );
    assert_eq!(parse_error_page_marker("network guess", SECRET), None);
    assert_eq!(
        parse_error_page_marker(&format!("other {SECRET}"), SECRET),
        None
    );
    assert_eq!(parse_error_page_marker("", SECRET), None);
    assert_eq!(parse_error_page_marker("network ", ""), None);
    assert!(ERROR_PAGE_PROBE_SCRIPT.contains("mercury-error-page"));
}

#[test]
fn continue_is_offered_only_in_the_top_level_page_and_for_local_names() {
    let page = certificate_error_page(SECRET, false);
    // Hidden until the script decides; a framed page (clickjacking) never shows it.
    assert!(page.contains("<div id=\"advanced\" hidden>"), "{page}");
    assert!(page.contains("window.top === window"), "{page}");
    assert!(page.contains("isLocalName(location.hostname)"), "{page}");
    // Automatic continuation (tests) obeys the same gate.
    let automatic = certificate_error_page(SECRET, true);
    let gate = automatic.find("if (!offered)").expect("the gate");
    let click = automatic.find("proceed.click()").expect("the click");
    assert!(gate < click);
}

/// Host names and addresses as the page has them (`location.hostname`), and whether they are on
/// this computer or the local network. The browser decides by `talaria_core::is_local_host`; the
/// page's `isLocalName` must give the same answers, and the script is run against these rows.
const LOCAL_NAMES: &[(&str, bool)] = &[
    ("localhost", true),
    ("app.localhost", true),
    ("printer.local.", true),
    ("nas", true),
    ("nas..", true),
    ("my-host.lan", true),
    ("router.home.arpa", true),
    ("home.arpa", false),
    ("intranet.example.com", false),
    ("127.8.9.10", true),
    ("0.0.0.0", true),
    ("0.1.2.3", false),
    ("0.0.0.10", false),
    ("192.168.1.1", true),
    ("192.169.1.1", false),
    ("169.255.0.1", false),
    ("255.255.255.255", false),
    ("10.0.0.1", true),
    ("172.16.0.1", true),
    ("172.32.0.1", false),
    ("100.64.0.1", true),
    ("100.128.0.1", false),
    ("169.254.1.1", true),
    ("8.8.8.8", false),
    ("[::1]", true),
    ("[::]", true),
    ("[fc00::1]", true),
    ("[fd12:3456::1]", true),
    ("[fc::1]", false),
    ("[fd00::]", true),
    ("[fe80::]", true),
    ("[fe80::1]", true),
    ("[febf::1]", true),
    ("[fe00::]", false),
    ("[::2]", false),
    ("[::ffff:a00:1]", false),
    ("[fec0::1]", false),
    ("[fe8::1]", false),
    ("[2001:db8::1]", false),
    ("example.com", false),
];

/// The page's `isLocalName` mirrors `talaria_core::is_local_host`, which decides in the browser.
/// The table pins the edge cases; the page must give the same answers (a mismatch fails safe,
/// but then "Continue anyway" shows and does nothing). This test pins the text of the script;
/// the tests at the end run it.
#[test]
fn the_page_and_the_browser_agree_on_local_names() {
    for (host, local) in LOCAL_NAMES {
        assert_eq!(talaria_core::is_local_host(host), *local, "{host}");
    }
    let page = certificate_error_page(SECRET, false);
    // Every trailing dot is ignored, as `is_local_name` trims them.
    assert!(
        page.contains("host.toLowerCase().replace(/[.]+$/, '')"),
        "{page}"
    );
    // Of 0.0.0.0/8 only 0.0.0.0, as `Ipv4Addr::is_unspecified`.
    assert!(page.contains("host === '0.0.0.0'"), "{page}");
    assert!(!page.contains("a === 0 "), "{page}");
    // IPv6 ranges by the first group's value, not by text ("[fc::1]" is 00fc::1).
    assert!(page.contains("(first & 0xfe00) === 0xfc00"), "{page}");
    assert!(page.contains("(first & 0xffc0) === 0xfe80"), "{page}");
    assert!(!page.contains("/^f[cd]/"), "{page}");
}

// The certificate page, run in boa_engine (a JavaScript engine written in Rust) with a small
// fake page: the script is the one `certificate_error_page` makes, nothing is re-typed here.

/// The script of the certificate page, the one that offers "Continue" (it holds `isLocalName`).
fn certificate_script(page: &str) -> String {
    page.split("<script>")
        .skip(1)
        .filter_map(|part| part.split_once("</script>").map(|(script, _)| script))
        .find(|script| script.contains("function isLocalName"))
        .expect("the certificate page has its script")
        .to_owned()
}

/// A fake page for the script: the elements it uses, `fetch` answering as the scenario says,
/// and a count of everything that could turn text into markup or leave the page.
const FAKE_PAGE: &str = r"
var log = { reloads: 0, back: 0, html: 0, fetches: [] };
var listeners = [];
function element(id, hidden) {
  var e = { id: id, hidden: hidden, disabled: false, onclick: null, _text: '' };
  Object.defineProperty(e, 'textContent', {
    get: function () { return e._text; },
    set: function (value) { e._text = String(value); }
  });
  ['innerHTML', 'outerHTML'].forEach(function (name) {
    Object.defineProperty(e, name, { get: function () { return ''; },
      set: function () { log.html++; } });
  });
  e.insertAdjacentHTML = function () { log.html++; };
  e.click = function () { if (e.onclick) { e.onclick(); } };
  return e;
}
var elements = { back: element('back', false), proceed: element('proceed', false),
  advanced: element('advanced', true), refusal: element('refusal', true) };
globalThis.document = {
  addEventListener: function (type, listener) { listeners.push([type, listener]); },
  getElementById: function (id) {
    if (!(id in elements)) { throw new Error('the page has no element ' + id); }
    return elements[id];
  },
  querySelector: function (selector) {
    if (selector.indexOf('mercury-certificate') >= 0) { return { content: scenario.certificate }; }
    if (selector.indexOf('mercury-error-page') >= 0) {
      return { content: 'certificate ' + scenario.secret };
    }
    throw new Error('the page has no element for ' + selector);
  },
  write: function () { log.html++; },
  writeln: function () { log.html++; }
};
globalThis.window = globalThis;
globalThis.top = scenario.framed ? {} : globalThis;
globalThis.history = { length: scenario.history, back: function () { log.back++; } };
globalThis.location = { hostname: scenario.hostname, host: scenario.hostname,
  reload: function () { log.reloads++; } };
globalThis.URLSearchParams = function (fields) {
  this.toString = function () {
    return Object.keys(fields).map(function (name) {
      return name + '=' + encodeURIComponent(fields[name]);
    }).join('&');
  };
};
globalThis.fetch = function (address) { log.fetches.push(String(address)); return scenario.answer(); };
function fire(type) {
  listeners.forEach(function (entry) { if (entry[0] === type) { entry[1](); } });
}
";

/// What the browser answers, as JavaScript for the fake `fetch`.
fn answer(status: u16, kind: &str, body: &str) -> String {
    format!(
        "function () {{ return Promise.resolve({{ ok: {status} >= 200 && {status} < 300, \
status: {status}, type: '{kind}', text: function () {{ return Promise.resolve({}); }} }}); }}",
        js_string(body)
    )
}

const OPAQUE_ANSWER: &str = "function () { return Promise.resolve({ ok: false, status: 0, \
type: 'opaque', text: function () { throw new Error('an opaque answer has no text'); } }); }";
const NETWORK_ERROR: &str =
    "function () { return Promise.reject(new Error('the request failed')); }";
const UNREADABLE_REFUSAL: &str = "function () { return Promise.resolve({ ok: false, status: 403, \
type: 'cors', text: function () { return Promise.reject(new Error('unreadable')); } }); }";

fn js_string(text: &str) -> String {
    let mut literal = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str("\\n"),
            other => literal.push(other),
        }
    }
    literal.push('"');
    literal
}

struct Scenario {
    hostname: &'static str,
    certificate: &'static str,
    framed: bool,
    history: u32,
    automatic: bool,
    answer: String,
}

impl Scenario {
    fn answering(answer: impl Into<String>) -> Self {
        Self {
            hostname: "nas.local",
            certificate: "QUJD",
            framed: false,
            history: 2,
            automatic: false,
            answer: answer.into(),
        }
    }
}

/// The page, loaded.
struct Page {
    context: Context,
}

impl Page {
    fn load(scenario: &Scenario) -> Self {
        let script = certificate_script(&certificate_error_page(SECRET, scenario.automatic));
        let mut page = Self {
            context: Context::default(),
        };
        let setup = format!(
            "globalThis.scenario = {{ hostname: {}, certificate: {}, secret: {}, framed: {}, \
history: {}, answer: {} }};",
            js_string(scenario.hostname),
            js_string(scenario.certificate),
            js_string(SECRET),
            scenario.framed,
            scenario.history,
            scenario.answer
        );
        page.run("the scenario", &setup);
        page.run("the fake page", FAKE_PAGE);
        page.run("the certificate script", &script);
        page.run("the load event", "fire('DOMContentLoaded');");
        page.context.run_jobs();
        page
    }

    fn run(&mut self, what: &str, source: &str) {
        if let Err(error) = self.context.eval(Source::from_bytes(source)) {
            panic!("{what}: {error}");
        }
    }

    fn read(&mut self, expression: &str) -> String {
        let value = self
            .context
            .eval(Source::from_bytes(&format!("String({expression})")))
            .unwrap_or_else(|error| panic!("{expression}: {error}"));
        value
            .to_string(&mut self.context)
            .expect("a string")
            .to_std_string_escaped()
    }

    /// The user presses "Continue anyway", and every answer arrives.
    fn click(&mut self) {
        self.run("the click", "elements.proceed.click();");
        self.context.run_jobs();
    }

    fn offered(&mut self) -> bool {
        self.read("elements.advanced.hidden") == "false"
    }
}

fn refusal_answer(code: &str) -> String {
    let status = if code == "malformed" { 400 } else { 403 };
    answer(status, "cors", code)
}

#[test]
fn an_accepted_answer_reloads_and_a_refused_one_does_not() {
    let mut page = Page::load(&Scenario::answering(answer(200, "cors", "accepted")));
    assert!(page.offered());
    page.click();
    assert_eq!(page.read("log.reloads"), "1");
    assert_eq!(page.read("log.fetches.length"), "1");
    // Mercury's request: the scheme, the run's secret, the host of the page, the certificate.
    let request = page.read("log.fetches[0]");
    assert_eq!(
        request,
        format!(
            "{CERTIFICATE_EXCEPTION_SCHEME}:allow?secret={SECRET}&host=nas.local&certificate=QUJD"
        )
    );
    // Nothing to explain: the page is gone with the reload.
    assert_eq!(page.read("elements.refusal.hidden"), "true");
    assert_eq!(page.read("log.html"), "0");

    // Any other success status counts as accepted.
    let mut page = Page::load(&Scenario::answering(answer(204, "cors", "")));
    page.click();
    assert_eq!(page.read("log.reloads"), "1");
}

#[test]
fn every_refusal_shows_its_line_re_enables_the_button_and_does_not_reload() {
    for code in ExceptionRefusal::CODES {
        let mut page = Page::load(&Scenario::answering(refusal_answer(code)));
        page.click();
        let line = certificate_refusal_line(code);
        assert_eq!(page.read("log.reloads"), "0", "{code}");
        assert_eq!(page.read("elements.refusal.hidden"), "false", "{code}");
        assert_eq!(page.read("elements.refusal.textContent"), line, "{code}");
        assert_eq!(page.read("elements.proceed.disabled"), "false", "{code}");
        assert_eq!(page.read("log.html"), "0", "{code}");
        assert!(!line.is_empty(), "{code}");
    }
}

#[test]
fn the_table_has_a_line_for_every_code_and_a_fallback_for_the_rest() {
    let page = certificate_error_page(SECRET, false);
    for code in ExceptionRefusal::CODES {
        let line = certificate_refusal_line(code);
        assert!(
            page.contains(&format!("'{code}': '{line}'")),
            "the page has no line for {code}: {page}"
        );
        // Plain text a script string and a sentence can hold: no quote, backslash or markup.
        assert!(
            !line.contains(['\'', '"', '\\', '<', '>', '&', '\n', '`', '$']),
            "{code}: {line}"
        );
    }
    let fallback = "Mercury could not continue.";
    for code in ["", "accepted", "a-new-code", "HSTS-HOST", "hsts-host "] {
        assert_eq!(certificate_refusal_line(code), fallback, "{code:?}");
    }
    assert!(page.contains(fallback));
    // What the user is told for the three engine answers that have something to say.
    assert!(certificate_refusal_line("hsts-host").contains("only with a valid certificate"));
    for code in ["certificate-changed", "no-failure-recorded"] {
        assert!(
            certificate_refusal_line(code).contains("Reload this page and check again"),
            "{code}"
        );
    }
    // No code is spelt two ways, and none is the accepted answer.
    let mut codes = ExceptionRefusal::CODES.to_vec();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), ExceptionRefusal::CODES.len());
    assert!(!codes.contains(&CERTIFICATE_EXCEPTION_ACCEPTED));
}

#[test]
fn the_text_of_an_answer_is_a_key_never_markup_and_never_shown() {
    let fallback = certificate_refusal_line("");
    for body in [
        "<img src=x onerror=alert(1)>",
        "</p><script>alert(1)</script>",
        "&lt;b&gt;",
        "' + alert(1) + '",
        "hsts-host<script>",
        "constructor",
        "__proto__",
        "toString",
        "hasOwnProperty",
        "valueOf",
        "",
        "   ",
    ] {
        let mut page = Page::load(&Scenario::answering(answer(403, "cors", body)));
        page.click();
        assert_eq!(page.read("log.reloads"), "0", "{body:?}");
        assert_eq!(
            page.read("elements.refusal.textContent"),
            fallback,
            "{body:?}"
        );
        assert_eq!(page.read("log.html"), "0", "{body:?}");
        assert_eq!(page.read("elements.proceed.disabled"), "false", "{body:?}");
    }
    // Surrounding white space around a real code is not part of it.
    let mut page = Page::load(&Scenario::answering(answer(403, "cors", " hsts-host\n")));
    page.click();
    assert_eq!(
        page.read("elements.refusal.textContent"),
        certificate_refusal_line("hsts-host")
    );
}

#[test]
fn the_script_has_no_way_to_make_markup_from_text() {
    let script = certificate_script(&certificate_error_page(SECRET, false));
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "createContextualFragment",
        "DOMParser",
        "eval(",
        "Function(",
        "setTimeout(",
        "setInterval(",
    ] {
        assert!(!script.contains(forbidden), "{forbidden}");
    }
    assert!(script.contains("refusal.textContent ="), "{script}");
    // The element the line goes in is hidden until there is one, and announced when it appears.
    let page = certificate_error_page(SECRET, false);
    assert!(
        page.contains("<p id=\"refusal\" role=\"alert\" hidden></p>"),
        "{page}"
    );
}

#[test]
fn an_answer_that_cannot_be_read_falls_back_to_reloading() {
    // A cross-origin answer without permission to read it: the page cannot tell, so it reloads,
    // which shows the same certificate page again after a refusal and the site after an accept.
    let mut page = Page::load(&Scenario::answering(OPAQUE_ANSWER));
    page.click();
    assert_eq!(page.read("log.reloads"), "1");
    assert_eq!(page.read("elements.refusal.hidden"), "true");

    // The request itself failed (a network error is also what a request without the secret gets).
    let mut page = Page::load(&Scenario::answering(NETWORK_ERROR));
    page.click();
    assert_eq!(page.read("log.reloads"), "1");
    assert_eq!(page.read("elements.refusal.hidden"), "true");
}

#[test]
fn a_refusal_whose_text_cannot_be_read_still_refuses() {
    let mut page = Page::load(&Scenario::answering(UNREADABLE_REFUSAL));
    page.click();
    assert_eq!(page.read("log.reloads"), "0");
    assert_eq!(page.read("elements.refusal.hidden"), "false");
    assert_eq!(
        page.read("elements.refusal.textContent"),
        certificate_refusal_line("")
    );
    assert_eq!(page.read("elements.proceed.disabled"), "false");
}

#[test]
fn the_button_works_again_after_a_refusal() {
    let mut page = Page::load(&Scenario::answering(refusal_answer("certificate-changed")));
    page.click();
    assert_eq!(page.read("elements.refusal.hidden"), "false");
    assert_eq!(page.read("elements.proceed.disabled"), "false");
    // The user reloads nothing and clicks again; this time the engine accepts.
    page.run(
        "the new answer",
        &format!("scenario.answer = {};", answer(200, "cors", "accepted")),
    );
    page.click();
    assert_eq!(page.read("log.fetches.length"), "2");
    assert_eq!(page.read("log.reloads"), "1");
    // The line of the first refusal is gone while the second request is out.
    assert_eq!(page.read("elements.refusal.hidden"), "true");
}

#[test]
fn continue_is_offered_by_the_script_only_in_the_top_level_page_for_local_names() {
    let offered = |scenario: Scenario| Page::load(&scenario).offered();
    assert!(offered(Scenario::answering(answer(
        200, "cors", "accepted"
    ))));
    for hostname in ["localhost", "127.0.0.1", "[::1]", "printer.local", "nas"] {
        let scenario = Scenario {
            hostname,
            ..Scenario::answering(answer(200, "cors", "accepted"))
        };
        assert!(offered(scenario), "{hostname}");
    }
    for hostname in ["example.com", "8.8.8.8", "[2001:db8::1]", "bank.example"] {
        let scenario = Scenario {
            hostname,
            ..Scenario::answering(answer(200, "cors", "accepted"))
        };
        assert!(!offered(scenario), "{hostname}");
    }
    let framed = Scenario {
        framed: true,
        ..Scenario::answering(answer(200, "cors", "accepted"))
    };
    assert!(!offered(framed));
    let without_certificate = Scenario {
        certificate: "",
        ..Scenario::answering(answer(200, "cors", "accepted"))
    };
    assert!(!offered(without_certificate));
    // A page that is not offered has no button that does anything.
    let mut page = Page::load(&Scenario {
        hostname: "example.com",
        ..Scenario::answering(answer(200, "cors", "accepted"))
    });
    page.click();
    assert_eq!(page.read("log.fetches.length"), "0");
    assert_eq!(page.read("log.reloads"), "0");
}

#[test]
fn automatic_continue_asks_once_and_obeys_the_gate_and_the_answer() {
    let automatic = |scenario: Scenario| Scenario {
        automatic: true,
        ..scenario
    };
    // Accepted: one request, one reload.
    let mut page = Page::load(&automatic(Scenario::answering(answer(
        200, "cors", "accepted",
    ))));
    assert_eq!(page.read("log.fetches.length"), "1");
    assert_eq!(page.read("log.reloads"), "1");
    // Refused: one request, the line, no reload and no second request.
    let mut page = Page::load(&automatic(Scenario::answering(refusal_answer("hsts-host"))));
    assert_eq!(page.read("log.fetches.length"), "1");
    assert_eq!(page.read("log.reloads"), "0");
    assert_eq!(
        page.read("elements.refusal.textContent"),
        certificate_refusal_line("hsts-host")
    );
    // The same gates as a click.
    let mut page = Page::load(&automatic(Scenario {
        framed: true,
        ..Scenario::answering(answer(200, "cors", "accepted"))
    }));
    assert_eq!(page.read("log.fetches.length"), "0");
    let mut page = Page::load(&automatic(Scenario {
        hostname: "example.com",
        ..Scenario::answering(answer(200, "cors", "accepted"))
    }));
    assert_eq!(page.read("log.fetches.length"), "0");
}

#[test]
fn go_back_is_offered_only_when_there_is_a_page_to_go_back_to() {
    let mut page = Page::load(&Scenario::answering(answer(200, "cors", "accepted")));
    assert_eq!(page.read("elements.back.hidden"), "false");
    page.run("the button", "elements.back.onclick();");
    assert_eq!(page.read("log.back"), "1");
    let mut page = Page::load(&Scenario {
        history: 1,
        ..Scenario::answering(answer(200, "cors", "accepted"))
    });
    assert_eq!(page.read("elements.back.hidden"), "true");
}

// The script's own rule for local names, run against the browser's.

/// `isLocalName` of the certificate page, in a context of its own.
fn local_name_context() -> Context {
    let script = certificate_script(&certificate_error_page(SECRET, false));
    // Only the function: the rest of the script needs a page.
    let start = script.find("function isLocalName").expect("the function");
    let end = start + script[start..].find("\n}\n").expect("its end") + 3;
    let mut context = Context::default();
    context
        .eval(Source::from_bytes(&script[start..end]))
        .expect("the function loads");
    context
}

fn script_says_local(context: &mut Context, host: &str) -> bool {
    context
        .eval(Source::from_bytes(&format!(
            "isLocalName({})",
            js_string(host)
        )))
        .expect("the answer")
        .to_boolean()
}

#[test]
fn the_script_decides_local_names_as_the_browser_does_for_every_table_row() {
    let mut context = local_name_context();
    for (host, local) in LOCAL_NAMES {
        assert_eq!(talaria_core::is_local_host(host), *local, "{host}");
        assert_eq!(script_says_local(&mut context, host), *local, "{host}");
    }
    // The page gets the host of its address, which `Url` has made canonical: upper-case never
    // arrives, but the rule does not depend on it.
    for host in ["LOCALHOST", "Printer.LOCAL.", "NAS", "Example.COM"] {
        assert_eq!(
            script_says_local(&mut context, host),
            talaria_core::is_local_host(host),
            "{host}"
        );
    }
}

#[test]
fn the_script_decides_every_ipv4_range_as_the_browser_does() {
    let mut context = local_name_context();
    let script_says = context
        .eval(Source::from_bytes(
            "(function () { var out = ''; for (var a = 0; a < 256; a++) { \
for (var b = 0; b < 256; b++) { out += isLocalName(a + '.' + b + '.1.1') ? '1' : '0'; } } \
return out; })()",
        ))
        .expect("the answers")
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped();
    let browser_says = (0..=255_u32)
        .flat_map(|a| (0..=255_u32).map(move |b| (a, b)))
        .map(|(a, b)| {
            if talaria_core::is_local_host(&format!("{a}.{b}.1.1")) {
                '1'
            } else {
                '0'
            }
        })
        .collect::<String>();
    assert_eq!(script_says.len(), browser_says.len());
    if let Some(index) = script_says
        .bytes()
        .zip(browser_says.bytes())
        .position(|(script, browser)| script != browser)
    {
        panic!(
            "{}.{}.1.1: the script says {}, the browser says {}",
            index / 256,
            index % 256,
            &script_says[index..=index],
            &browser_says[index..=index]
        );
    }
}

#[test]
fn the_script_decides_every_ipv6_range_as_the_browser_does() {
    let mut context = local_name_context();
    // The first group decides; the address of a page is canonical (`[::1]` rather than
    // `[0::1]`), so the first group runs from 1 and `[::1]` is in the table.
    let script_says = context
        .eval(Source::from_bytes(
            "(function () { var out = ''; for (var g = 1; g < 65536; g++) { \
out += isLocalName('[' + g.toString(16) + '::1]') ? '1' : '0'; } return out; })()",
        ))
        .expect("the answers")
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped();
    let browser_says = (1..=0xffff_u32)
        .map(|group| {
            if talaria_core::is_local_host(&format!("[{group:x}::1]")) {
                '1'
            } else {
                '0'
            }
        })
        .collect::<String>();
    assert_eq!(script_says.len(), browser_says.len());
    if let Some(index) = script_says
        .bytes()
        .zip(browser_says.bytes())
        .position(|(script, browser)| script != browser)
    {
        panic!(
            "[{:x}::1]: the script says {}, the browser says {}",
            index + 1,
            &script_says[index..=index],
            &browser_says[index..=index]
        );
    }
}
