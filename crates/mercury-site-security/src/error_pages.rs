// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Mercury's error pages, served to the engine in place of its own (`Resource::BadCertHTML`,
//! `NetErrorHTML` and `CrashHTML` in Servo 0.8).
//!
//! The engine fills each template by plain text replacement: `${reason}` (and, for the
//! certificate page, `${bytes}`, the certificate in base64) on every page, `${details}` on the
//! crash page. A reason can carry text a server chose (a name from its certificate), so it is
//! never placed where it could become markup: every template ends with a `<plaintext>` element
//! holding it, which no later text can close, and a script copies it into the page as text.
//!
//! Each page also carries a marker, `<meta name="mercury-error-page" content="<kind> <secret>">`,
//! where `<secret>` is a random value the browser makes at start-up. The browser recognises its
//! error pages by it (`ERROR_PAGE_PROBE_SCRIPT`, `parse_error_page_marker`); a web page does not
//! know the value and cannot pass for one. The certificate page sends the same value with the
//! user's "Continue" to `CERTIFICATE_EXCEPTION_SCHEME`, which is how the browser knows the
//! request came from its own page.
//!
//! The browser answers that request (see `exception_request`): a success status when it accepted
//! the certificate, and the page reloads, or a refusal with a code as the body, and the page shows
//! one line from its own fixed table (`certificate_refusal_line`) and does not reload. The text
//! of an answer is only ever a key of that table, never shown and never made into markup. If the
//! answer cannot be read at all, or the request fails, the page reloads whatever happened, which
//! shows the same page again after a refusal.

/// The scheme of the address the certificate page asks for when the user continues:
/// `<scheme>:allow?secret=…&host=…&certificate=…` (the certificate in base64, no padding).
pub const CERTIFICATE_EXCEPTION_SCHEME: &str = "mercury-certificate-exception";

/// What the certificate page says when the browser refuses an exception, by the code of the
/// refusal (`ExceptionRefusal::code`). Plain sentences: they are placed in the page's script as
/// strings and in the page as text, never as markup.
const REFUSAL_LINES: [(&str, &str); 8] = [
    ("not-from-mercury", REFUSAL_FALLBACK),
    ("malformed", REFUSAL_FALLBACK),
    (
        "public-host",
        "Mercury lets you continue only to sites on this computer or your local network.",
    ),
    ("invalid-host", REFUSAL_FALLBACK),
    (
        "hsts-host",
        "This site asked to be reached only with a valid certificate, so Mercury cannot continue.",
    ),
    (
        "no-failure-recorded",
        "Mercury has no record of this certificate failing. Reload this page and check again.",
    ),
    (
        "certificate-changed",
        "The certificate of this site changed after this page was shown. Reload this page and check again.",
    ),
    ("refused", REFUSAL_FALLBACK),
];

/// The line for a code the table does not have, and for the codes that have nothing more to say.
const REFUSAL_FALLBACK: &str = "Mercury could not continue.";

/// The line the certificate page shows for the refusal `code` (the body of the browser's answer);
/// `Mercury could not continue.` for a code it does not know.
pub fn certificate_refusal_line(code: &str) -> &'static str {
    REFUSAL_LINES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(REFUSAL_FALLBACK, |(_, line)| line)
}

/// `text` as a single-quoted script string that can sit in a `<script>` element: quotes,
/// backslashes, angle brackets, ampersands and line breaks are escaped.
fn script_string(text: &str) -> String {
    let mut literal = String::from("'");
    for character in text.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '\'' => literal.push_str("\\'"),
            '<' => literal.push_str("\\u003c"),
            '>' => literal.push_str("\\u003e"),
            '&' => literal.push_str("\\u0026"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\u{2028}' => literal.push_str("\\u2028"),
            '\u{2029}' => literal.push_str("\\u2029"),
            other => literal.push(other),
        }
    }
    literal.push('\'');
    literal
}

/// The script's table of lines, `{ 'code': 'line', … }`, from `REFUSAL_LINES`.
fn refusal_lines_script() -> String {
    let entries = REFUSAL_LINES
        .iter()
        .map(|(code, line)| format!("  {}: {}", script_string(code), script_string(line)))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n{entries}\n}}")
}

/// Script the browser runs in a finished page: returns the marker's content, or "".
pub const ERROR_PAGE_PROBE_SCRIPT: &str = "(function () { var marker = \
document.querySelector('meta[name=\"mercury-error-page\"]'); \
return marker ? String(marker.content) : ''; })()";

/// Which of Mercury's error pages a page is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorPage {
    Network,
    Certificate,
}

/// The error page whose marker `content` is, if it carries `secret`.
pub fn parse_error_page_marker(content: &str, secret: &str) -> Option<ErrorPage> {
    let (kind, marker_secret) = content.split_once(' ')?;
    if secret.is_empty() || !constant_time_equal(marker_secret, secret) {
        return None;
    }
    match kind {
        "network" => Some(ErrorPage::Network),
        "certificate" => Some(ErrorPage::Certificate),
        _ => None,
    }
}

fn constant_time_equal(first: &str, second: &str) -> bool {
    first.len() == second.len()
        && first
            .bytes()
            .zip(second.bytes())
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

const STYLE: &str = "<style>
:root { color-scheme: light dark; --text: #141a21; --secondary: #5a6573; --background: #f4f6f8;
  --card: #ffffff; --border: #d6dce3; --accent: #1683d8; --accent-text: #ffffff; --warning: #b3261e; }
@media (prefers-color-scheme: dark) {
  :root { --text: #edf4fa; --secondary: #8c99a9; --background: #0c1117; --card: #141b24;
    --border: #29323d; --accent: #62bfff; --accent-text: #09131c; --warning: #ff9e94; }
}
html { background: var(--background); color: var(--text);
  font: 15px/1.5 -apple-system, system-ui, sans-serif; }
body { margin: 0; padding: 12vh 24px 48px; }
main { max-width: 560px; margin: 0 auto; background: var(--card); border: 1px solid var(--border);
  border-radius: 12px; padding: 32px; }
h1 { font-size: 22px; line-height: 1.3; margin: 0 0 12px; }
.warning h1 { color: var(--warning); }
p { margin: 0 0 12px; color: var(--secondary); }
#host { color: var(--text); font-weight: 600; word-break: break-all; }
.actions { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 24px; }
button { font: inherit; border-radius: 8px; padding: 7px 16px; border: 1px solid var(--border);
  background: transparent; color: var(--text); cursor: pointer; }
button.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
details { margin-top: 24px; color: var(--secondary); }
summary { cursor: pointer; }
#reason { font: 12px/1.4 ui-monospace, monospace; white-space: pre-wrap; word-break: break-all;
  margin: 8px 0 0; }
#refusal { color: var(--warning); margin: 12px 0 0; }
[hidden] { display: none !important; }
</style>";

/// Copies the text after `<plaintext>` into `#reason` and fills `#host`.
const FILL_SCRIPT: &str = "<script>
document.addEventListener('DOMContentLoaded', function () {
  var held = document.querySelector('plaintext');
  var reason = held ? held.textContent.trim() : '';
  if (held) { held.remove(); }
  document.getElementById('reason').textContent = reason || 'No further details.';
  document.getElementById('host').textContent = location.host || location.href;
});
</script>";

/// The page for a certificate the engine did not accept. `proceed_automatically` (tests only)
/// presses "Continue" as soon as the page has loaded.
pub fn certificate_error_page(secret: &str, proceed_automatically: bool) -> String {
    let secret = attribute_safe(secret);
    let refusal_lines = refusal_lines_script();
    let fallback_line = script_string(REFUSAL_FALLBACK);
    format!(
        "<!DOCTYPE html>
<html lang=\"en\" class=\"warning\">
<head>
<meta charset=\"utf-8\">
<meta name=\"mercury-error-page\" content=\"certificate {secret}\">
<meta name=\"mercury-certificate\" content=\"${{bytes}}\">
<title>Certificate not valid</title>
{STYLE}
{FILL_SCRIPT}
<script>
// The browser accepts exceptions only for names on this computer or the local network
// (`talaria_core::certificate_exception_allowed`, which applies `is_local_host`, checked again
// where the request arrives); this mirrors the rule for local names.
function isLocalName(host) {{
  // A name may end in dots (`nas..` is valid); the browser's rule ignores all of them.
  host = host.toLowerCase().replace(/[.]+$/, '');
  if (host === 'localhost') {{ return true; }}
  var suffixes = ['.localhost', '.local', '.internal', '.lan', '.home.arpa'];
  if (suffixes.some(function (suffix) {{ return host.slice(-suffix.length) === suffix; }})) {{
    return true;
  }}
  var parts = host.split('.');
  if (parts.length === 4 && parts.every(function (part) {{ return /^[0-9]+$/.test(part); }})) {{
    // Loopback, private, link-local and shared (100.64/10) ranges; of 0.0.0.0/8 only 0.0.0.0.
    var a = +parts[0], b = +parts[1];
    return host === '0.0.0.0' || a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168) || (a === 169 && b === 254) || (a === 100 && b >= 64 && b <= 127);
  }}
  if (host.charAt(0) === '[') {{
    // The serialised address: `::` for zero groups, no leading zeros. Compare the first group's
    // value (fc00::/7 unique local, fe80::/10 link-local), not its text.
    var v6 = host.slice(1, -1);
    var first = parseInt(v6.split(':')[0] || '0', 16);
    return v6 === '::1' || v6 === '::' || (first & 0xfe00) === 0xfc00 ||
      (first & 0xffc0) === 0xfe80;
  }}
  return host.indexOf('.') < 0;
}}
// What the page says when the browser refuses, by the code in the body of its answer. The text of
// an answer is only ever looked up here, never shown and never made into markup.
var refusalLines = {refusal_lines};
function refusalLine(code) {{
  return Object.prototype.hasOwnProperty.call(refusalLines, code) ? refusalLines[code] :
    {fallback_line};
}}
document.addEventListener('DOMContentLoaded', function () {{
  var certificate = document.querySelector('meta[name=\"mercury-certificate\"]').content;
  var secret = document.querySelector('meta[name=\"mercury-error-page\"]').content.split(' ')[1];
  var back = document.getElementById('back');
  var proceed = document.getElementById('proceed');
  var refusal = document.getElementById('refusal');
  if (history.length > 1) {{
    back.onclick = function () {{ history.back(); }};
  }} else {{
    back.hidden = true;
  }}
  // Never in a frame (a page could hide this one and lure the clicks), never for a public
  // site, and never without the certificate.
  var offered = !!certificate && window.top === window && isLocalName(location.hostname);
  if (!offered) {{ return; }}
  document.getElementById('advanced').hidden = false;
  proceed.onclick = function () {{
    proceed.disabled = true;
    refusal.hidden = true;
    var query = new URLSearchParams({{ secret: secret, host: location.hostname,
      certificate: certificate }});
    var reload = function () {{ location.reload(); }};
    var refuse = function (code) {{
      refusal.textContent = refusalLine(code);
      refusal.hidden = false;
      proceed.disabled = false;
    }};
    fetch('{CERTIFICATE_EXCEPTION_SCHEME}:allow?' + query.toString()).then(function (response) {{
      // Accepted: reload, and the page is the site. An answer the page cannot read (status 0,
      // an opaque cross-origin answer) is not a refusal it knows of: reload, as when the request
      // failed (the second function), which shows this page again if the browser refused.
      if (response.ok || response.status === 0) {{ reload(); return; }}
      return response.text().then(function (code) {{ refuse(String(code).trim()); }},
        function () {{ refuse(''); }});
    }}, reload);
  }};
  if ({proceed_automatically}) {{ proceed.click(); }}
}});
</script>
</head>
<body>
<main>
<h1>This site’s certificate is not valid</h1>
<p>Mercury Browser could not confirm that this is <span id=\"host\"></span>. Someone may be
trying to read or change what you send to and receive from it, such as passwords or card
numbers.</p>
<div class=\"actions\"><button id=\"back\" class=\"primary\">Go back</button></div>
<details>
<summary>Details</summary>
<p id=\"reason\"></p>
<div id=\"advanced\" hidden>
<p>This site is on your computer or your local network. If you know why its certificate is not
valid, for example because it is your own test server, you can continue anyway for the rest of
this session.</p>
<div class=\"actions\"><button id=\"proceed\">Continue anyway (not secure)</button></div>
<p id=\"refusal\" role=\"alert\" hidden></p>
</div>
</details>
</main>
<plaintext hidden>${{reason}}"
    )
}

/// The page for a page that could not be loaded.
pub fn network_error_page(secret: &str) -> String {
    let secret = attribute_safe(secret);
    format!(
        "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<meta name=\"mercury-error-page\" content=\"network {secret}\">
<title>Page not loaded</title>
{STYLE}
{FILL_SCRIPT}
<script>
document.addEventListener('DOMContentLoaded', function () {{
  document.getElementById('retry').onclick = function () {{ location.reload(); }};
}});
</script>
</head>
<body>
<main>
<h1>This page could not be loaded</h1>
<p>Mercury Browser could not load the page at <span id=\"host\"></span>. The site may be down,
or your network connection may have stopped working.</p>
<div class=\"actions\"><button id=\"retry\" class=\"primary\">Try again</button></div>
<details>
<summary>Details</summary>
<p id=\"reason\"></p>
</details>
</main>
<plaintext hidden>${{reason}}"
    )
}

/// The page shown when the engine's renderer for a page failed.
pub fn crash_page() -> String {
    format!(
        "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>Page stopped working</title>
{STYLE}
{FILL_SCRIPT}
<script>
document.addEventListener('DOMContentLoaded', function () {{
  document.getElementById('retry').onclick = function () {{ location.reload(); }};
}});
</script>
</head>
<body>
<main>
<h1>This page stopped working</h1>
<p>Something went wrong while Mercury Browser was showing <span id=\"host\"></span>.</p>
<div class=\"actions\"><button id=\"retry\" class=\"primary\">Reload</button></div>
<details>
<summary>Details</summary>
<p id=\"reason\"></p>
</details>
</main>
<plaintext hidden>${{details}}"
    )
}

/// The secret is made by the browser from hexadecimal digits; anything else is dropped so the
/// value can never end the attribute it sits in.
fn attribute_safe(secret: &str) -> String {
    secret.chars().filter(char::is_ascii_alphanumeric).collect()
}

#[cfg(test)]
mod tests {
    use super::{REFUSAL_LINES, script_string};
    use crate::ExceptionRefusal;

    #[test]
    fn the_table_has_the_codes_of_the_refusals_in_their_order() {
        let codes = REFUSAL_LINES
            .iter()
            .map(|(code, _)| *code)
            .collect::<Vec<_>>();
        assert_eq!(codes, ExceptionRefusal::CODES);
    }

    #[test]
    fn a_script_string_cannot_end_its_string_or_its_script() {
        assert_eq!(script_string("plain text."), "'plain text.'");
        let hostile = "it's \\ </script><b>&\n\u{2028}";
        let literal = script_string(hostile);
        for forbidden in ["</", "<", ">", "&", "\n", "\u{2028}"] {
            assert!(!literal.contains(forbidden), "{forbidden}: {literal}");
        }
        // Every quote inside is escaped, and the string is closed once, at its end.
        let inner = &literal[1..literal.len() - 1];
        assert!(literal.starts_with('\'') && literal.ends_with('\''));
        assert_eq!(inner.matches('\'').count(), inner.matches("\\'").count());
        assert_eq!(
            literal,
            "'it\\'s \\\\ \\u003c/script\\u003e\\u003cb\\u003e\\u0026\\n\\u2028'"
        );
    }
}
