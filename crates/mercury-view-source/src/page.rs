// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The page that shows a source. Item 1 (source-page) fills in the bodies; the rules are in
//! `sdd/lane-sh-pages-brief.md`, "Rules the two halves keep".
//!
//! The page is built as one string with no script at all. Three things keep it safe:
//!
//! - **One escape function** ([`escape_into`]) writes every character that came from the source
//!   or from a server (text, content type, URLs, attribute values). It escapes `& < > " '` and
//!   turns control characters into their visible "control picture" (NUL becomes `U+2400`), so
//!   nothing a source holds can be read as markup, as an attribute, or as a NUL.
//! - **A fixed set of tags**: `html head meta title style body header h1 p a label input main div
//!   span`. The generator writes tag names as literals only; a source can add none.
//! - **A content security policy** in [`page_headers`] that allows no script, no image, no frame,
//!   no form and no `<base>`; the style sheet is the one inline block.
//!
//! [`render_page`] blocks the calling thread while the highlighter runs (at most about 2.5 s; see
//! `highlight.rs`), so a caller on an async runtime calls it from a blocking thread.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};

use url::Url;

use crate::address::ViewSourceUrl;
use crate::fetch::FetchError;
use crate::highlight::{self, CLASSES, Highlighted, Outcome, Span};
use crate::source::{SourceBody, SourceDocument, SourceKind};
use crate::style::{MAX_GUTTER_DIGITS, stylesheet};
use crate::{MAX_HIGHLIGHT_BYTES, MAX_LINKS, MAX_SHOWN_BYTES, MAX_SHOWN_LINES};

/// A generated page: self-contained HTML, no script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSourcePage {
    /// The document.
    pub html: String,
}

/// The sentence every source page carries (the brief gives its words).
const FRESH_COPY_NOTE: &str =
    "This is a fresh copy fetched from the server, so it can differ from the page you see.";
/// The longest `<title>`, in characters.
const TITLE_LIMIT: usize = 60;
/// The most bytes of `view-source:` addresses one page carries in its links: with 5,000 links of
/// up to 8 KiB each the page could otherwise grow far past the size the tests bound.
const MAX_LINK_BYTES: usize = 2 * 1024 * 1024;
/// What is served if building the page panicked twice (it cannot reach the network or the engine).
const FALLBACK_PAGE: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<title>Source</title>\n</head>\n<body>\n<header>\n<h1>View source</h1>\n\
<p class=\"err\">The source could not be shown.</p>\n</header>\n</body>\n</html>\n";

/// The page for a fetched source. Never fails: whatever cannot be highlighted is shown plain.
pub fn render_page(document: &SourceDocument, request: &ViewSourceUrl) -> ViewSourcePage {
    let target = request.target();
    let html = catch_unwind(AssertUnwindSafe(|| build_page(document, target, true)))
        .or_else(|_| catch_unwind(AssertUnwindSafe(|| build_page(document, target, false))))
        .unwrap_or_else(|_| FALLBACK_PAGE.to_owned());
    ViewSourcePage { html }
}

/// The page for a fetch that failed (same look, one sentence from [`FetchError::summary`]).
pub fn render_error_page(request: &ViewSourceUrl, error: &FetchError) -> ViewSourcePage {
    let target = request.target();
    let html = catch_unwind(AssertUnwindSafe(|| build_error_page(target, error)))
        .unwrap_or_else(|_| FALLBACK_PAGE.to_owned());
    ViewSourcePage { html }
}

/// The response headers of every page: `content-type`, `content-security-policy`,
/// `cache-control: no-store`, `x-content-type-options: nosniff`, `referrer-policy: no-referrer`.
pub fn page_headers() -> [(&'static str, &'static str); 5] {
    [
        ("content-type", "text/html; charset=utf-8"),
        (
            "content-security-policy",
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; \
             form-action 'none'; frame-ancestors 'none'",
        ),
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ]
}

// ---------------------------------------------------------------------------------------------
// Escaping, and the small formatting helpers.
// ---------------------------------------------------------------------------------------------

/// Appends `text` as HTML text or as the value of a quoted attribute.
///
/// This is the only way anything from a source or a server reaches the page. `& < > " '` become
/// entities. Control characters other than the tab (they would be dropped or rewritten by an HTML
/// parser, and a NUL is a parse error) become the matching "control picture" (`U+2400` plus the
/// code; `DEL` is `U+2421`), so they stay visible in a source viewer. Every other character,
/// including `U+2028` and the byte order mark, is copied as it is.
pub(crate) fn escape_into(out: &mut String, text: &str) {
    let mut run_start = 0;
    for (index, &byte) in text.as_bytes().iter().enumerate() {
        let entity = match byte {
            b'&' => Some("&amp;"),
            b'<' => Some("&lt;"),
            b'>' => Some("&gt;"),
            b'"' => Some("&quot;"),
            b'\'' => Some("&#39;"),
            _ => None,
        };
        let picture = match byte {
            0x00..=0x08 | 0x0a..=0x1f => char::from_u32(0x2400 + u32::from(byte)),
            0x7f => Some('\u{2421}'),
            _ => None,
        };
        if entity.is_none() && picture.is_none() {
            continue;
        }
        // Every byte matched above is ASCII, so `index` is on a character boundary.
        out.push_str(&text[run_start..index]);
        if let Some(entity) = entity {
            out.push_str(entity);
        } else if let Some(picture) = picture {
            out.push(picture);
        }
        run_start = index + 1;
    }
    out.push_str(&text[run_start..]);
}

/// The text with its HTML character references decoded, as far as links need it: the five named
/// references that attribute values use in practice (`&amp; &lt; &gt; &quot; &apos;`) and numeric
/// ones. Anything else (an unknown name, no semicolon) is left as it is.
fn decode_entities(raw: &str) -> Cow<'_, str> {
    if !raw.contains('&') {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let semicolon = rest
            .as_bytes()
            .iter()
            .take(12)
            .position(|&byte| byte == b';');
        let decoded = semicolon.and_then(|semicolon| {
            let name = &rest[1..semicolon];
            let character = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name.strip_prefix('#').and_then(|number| {
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code).filter(|c| *c != '\0')
                }),
            };
            character.map(|character| (character, semicolon + 1))
        });
        match decoded {
            Some((character, used)) => {
                out.push(character);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// `1234567` as `1,234,567`.
fn with_commas(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `512 bytes`, `12.3 KiB`, `2.0 MiB`.
#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes < KIB {
        format!("{bytes} bytes")
    } else if bytes < MIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

/// `12.3 KiB (12,596 bytes)`; a size under 1 KiB is only `512 bytes`.
fn format_size_exact(bytes: u64) -> String {
    if bytes < 1024 {
        format_size(bytes)
    } else {
        format!("{} ({} bytes)", format_size(bytes), with_commas(bytes))
    }
}

/// The reason phrase (with its leading space) of the statuses a page is likely to have.
const fn status_text(status: u16) -> &'static str {
    match status {
        200 => " OK",
        201 => " Created",
        202 => " Accepted",
        203 => " Non-Authoritative Information",
        204 => " No Content",
        206 => " Partial Content",
        300 => " Multiple Choices",
        301 => " Moved Permanently",
        302 => " Found",
        303 => " See Other",
        304 => " Not Modified",
        307 => " Temporary Redirect",
        308 => " Permanent Redirect",
        400 => " Bad Request",
        401 => " Unauthorized",
        403 => " Forbidden",
        404 => " Not Found",
        405 => " Method Not Allowed",
        406 => " Not Acceptable",
        408 => " Request Timeout",
        410 => " Gone",
        418 => " I'm a teapot",
        429 => " Too Many Requests",
        451 => " Unavailable For Legal Reasons",
        500 => " Internal Server Error",
        501 => " Not Implemented",
        502 => " Bad Gateway",
        503 => " Service Unavailable",
        504 => " Gateway Timeout",
        _ => "",
    }
}

/// `Source of <host><path>`, at most [`TITLE_LIMIT`] characters (the last is `…` when it was cut).
fn page_title(target: &Url) -> String {
    let full = format!(
        "Source of {}{}",
        target.host_str().unwrap_or(""),
        target.path()
    );
    if full.chars().count() <= TITLE_LIMIT {
        return full;
    }
    let mut cut: String = full.chars().take(TITLE_LIMIT - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// The address of a page as shown: no credentials, no fragment.
fn shown_address(url: &Url) -> String {
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    url.into()
}

/// `view-source:<url>` for an `http(s)` address (credentials and fragment dropped, at most
/// [`crate::MAX_ADDRESS_BYTES`] long); `None` for any other.
fn source_address(url: &Url) -> Option<String> {
    ViewSourceUrl::for_page(url).map(|address| address.to_url().into())
}

/// The `view-source:` address an `href` or `src` value of the source points at, resolved against
/// the address the page was loaded from, by `ViewSourceUrl::link_to`. `<base>` is ignored. A bare
/// `#fragment` is not a link: it would lead to the page that is already shown.
fn resolve_link(base: &ViewSourceUrl, raw: &str) -> Option<String> {
    let value = decode_entities(raw);
    let value = value.trim_matches(|c: char| c.is_ascii_whitespace() || c.is_ascii_control());
    if value.is_empty() || value.starts_with('#') {
        return None;
    }
    base.link_to(value).map(|address| address.to_url().into())
}

// ---------------------------------------------------------------------------------------------
// Lines.
// ---------------------------------------------------------------------------------------------

/// The line that starts at `pos`: `(start, end, next)` where `end` leaves out the terminator
/// (`\n`, `\r\n` or a lone `\r`) and `next` is where the following line starts. `None` at the end.
fn next_line(text: &str, pos: usize) -> Option<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    if pos >= bytes.len() {
        return None;
    }
    match bytes[pos..].iter().position(|&b| b == b'\n' || b == b'\r') {
        None => Some((pos, bytes.len(), bytes.len())),
        Some(offset) => {
            let end = pos + offset;
            let terminator = if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            Some((pos, end, end + terminator))
        }
    }
}

fn count_lines(text: &str) -> usize {
    let mut count = 0;
    let mut pos = 0;
    while let Some((_, _, next)) = next_line(text, pos) {
        count += 1;
        pos = next;
    }
    count
}

fn digits_of(number: usize) -> usize {
    number.max(1).ilog10() as usize + 1
}

/// A link on a range of the text.
struct LinkRange {
    start: usize,
    end: usize,
    /// The `view-source:` address.
    href: String,
}

/// Turns the highlighter's raw `href` and `src` ranges into links, at most [`MAX_LINKS`], within
/// [`MAX_LINK_BYTES`] of address text.
fn make_links(text: &str, base: &ViewSourceUrl, raw: &[Range<usize>]) -> Vec<LinkRange> {
    let mut links = Vec::new();
    let mut budget = MAX_LINK_BYTES;
    for range in raw {
        if links.len() >= MAX_LINKS {
            break;
        }
        let Some(value) = text.get(range.clone()) else {
            continue;
        };
        let Some(href) = resolve_link(base, value) else {
            continue;
        };
        if href.len() > budget {
            break;
        }
        budget -= href.len();
        links.push(LinkRange {
            start: range.start,
            end: range.end,
            href,
        });
    }
    links
}

/// Writes the content of one line: its text, with the spans and links that cover it. A span or a
/// link that goes on over a line break is closed and opened again on the next line, so that every
/// line is a whole piece of markup. `next_span` and `next_link` carry over from line to line.
fn push_line_content(
    out: &mut String,
    text: &str,
    (start, end): (usize, usize),
    spans: &[Span],
    next_span: &mut usize,
    links: &[LinkRange],
    next_link: &mut usize,
) {
    let mut open_link: Option<usize> = None;
    let mut pos = start;
    while pos < end {
        while *next_span < spans.len() && spans[*next_span].end <= pos {
            *next_span += 1;
        }
        while *next_link < links.len() && links[*next_link].end <= pos {
            *next_link += 1;
        }
        let (class, class_end) = match spans.get(*next_span) {
            Some(span) if span.start <= pos => (Some(span.class), span.end),
            Some(span) => (None, span.start),
            None => (None, usize::MAX),
        };
        let (link, link_end) = match links.get(*next_link) {
            Some(link) if link.start <= pos => (Some(*next_link), link.end),
            Some(link) => (None, link.start),
            None => (None, usize::MAX),
        };
        // Both ends are after `pos` (finished ones were skipped above), so this always advances.
        let piece_end = end.min(class_end).min(link_end);
        if open_link != link {
            if open_link.is_some() {
                out.push_str("</a>");
            }
            if let Some(index) = link {
                out.push_str("<a href=\"");
                escape_into(out, &links[index].href);
                out.push_str("\">");
            }
            open_link = link;
        }
        if let Some(piece) = text.get(pos..piece_end) {
            match class.and_then(|class| CLASSES.get(usize::from(class))) {
                Some(class) => {
                    out.push_str("<span class=\"");
                    out.push_str(class.css);
                    out.push_str("\">");
                    escape_into(out, piece);
                    out.push_str("</span>");
                }
                None => escape_into(out, piece),
            }
        }
        pos = piece_end;
    }
    if open_link.is_some() {
        out.push_str("</a>");
    }
}

/// Writes the lines, at most `max_lines`; returns how many were written.
fn push_lines(
    out: &mut String,
    text: &str,
    spans: &[Span],
    links: &[LinkRange],
    max_lines: usize,
) -> usize {
    let mut pos = 0;
    let mut number = 0;
    let mut next_span = 0;
    let mut next_link = 0;
    while number < max_lines {
        let Some((start, end, next)) = next_line(text, pos) else {
            break;
        };
        number += 1;
        let _ = write!(out, "<div class=\"l\" data-n=\"{number}\"><span>");
        push_line_content(
            out,
            text,
            (start, end),
            spans,
            &mut next_span,
            links,
            &mut next_link,
        );
        out.push_str("</span></div>");
        pos = next;
    }
    number
}

// ---------------------------------------------------------------------------------------------
// The page.
// ---------------------------------------------------------------------------------------------

/// What the header shows.
struct Header<'a> {
    target: &'a Url,
    /// The address after redirects, when it is another one.
    redirected_to: Option<&'a Url>,
    facts: Vec<String>,
    /// Shown for a source: the page is a fresh copy.
    fresh_copy: bool,
    notices: Vec<String>,
    error: Option<&'static str>,
    wrap_toggle: bool,
}

fn push_page_start(out: &mut String, target: &Url, wrap_toggle: bool) {
    out.push_str(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n<title>",
    );
    escape_into(out, &page_title(target));
    out.push_str("</title>\n<style>");
    out.push_str(stylesheet());
    out.push_str("</style>\n</head>\n<body>\n");
    if wrap_toggle {
        // Before the header and the lines, so that sibling selectors reach both. Wrapping is on.
        out.push_str("<input type=\"checkbox\" id=\"wrap\" checked>\n");
    }
}

fn push_page_end(out: &mut String) {
    out.push_str("</body>\n</html>\n");
}

fn push_header(out: &mut String, header: &Header<'_>) {
    out.push_str("<header>\n<h1>View source</h1>\n<p class=\"addr\">");
    escape_into(out, &shown_address(header.target));
    out.push_str("</p>\n");
    if let Some(final_url) = header.redirected_to {
        out.push_str("<p class=\"addr\">Redirected to ");
        let shown = shown_address(final_url);
        match source_address(final_url) {
            Some(href) => {
                out.push_str("<a href=\"");
                escape_into(out, &href);
                out.push_str("\">");
                escape_into(out, &shown);
                out.push_str("</a>");
            }
            None => escape_into(out, &shown),
        }
        out.push_str("</p>\n");
    }
    if !header.facts.is_empty() {
        out.push_str("<p class=\"facts\">");
        for fact in &header.facts {
            out.push_str("<span>");
            escape_into(out, fact);
            out.push_str("</span>");
        }
        out.push_str("</p>\n");
    }
    if header.fresh_copy {
        out.push_str("<p class=\"note\">");
        escape_into(out, FRESH_COPY_NOTE);
        out.push_str("</p>\n");
    }
    for notice in &header.notices {
        out.push_str("<p class=\"notice\">");
        escape_into(out, notice);
        out.push_str("</p>\n");
    }
    if let Some(error) = header.error {
        out.push_str("<p class=\"err\">");
        escape_into(out, error);
        out.push_str("</p>\n");
    }
    if header.wrap_toggle {
        out.push_str("<label for=\"wrap\">Wrap long lines</label>\n");
    }
    out.push_str("</header>\n");
}

const fn has_grammar(kind: SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::Html | SourceKind::Xml | SourceKind::JavaScript | SourceKind::Css
    )
}

fn content_type_fact(content_type: Option<&str>) -> String {
    match content_type {
        Some(value) if !value.trim().is_empty() => format!("Content type: {}", value.trim()),
        _ => "Content type: not sent".to_owned(),
    }
}

fn status_fact(status: u16) -> String {
    format!("Status: {status}{}", status_text(status))
}

fn build_error_page(target: &Url, error: &FetchError) -> String {
    let mut out = String::with_capacity(8 * 1024);
    push_page_start(&mut out, target, false);
    push_header(
        &mut out,
        &Header {
            target,
            redirected_to: None,
            facts: Vec::new(),
            fresh_copy: false,
            notices: Vec::new(),
            error: Some(error.summary()),
            wrap_toggle: false,
        },
    );
    push_page_end(&mut out);
    out
}

/// The whole page for a fetched document. `colour` false skips the highlighter (the second try,
/// after a panic in the first).
fn build_page(document: &SourceDocument, target: &Url, colour: bool) -> String {
    let mut out = String::with_capacity(64 * 1024);
    // Compared as shown: a fragment or credentials alone are not a redirect.
    let redirected_to = (shown_address(&document.final_url) != shown_address(target))
        .then_some(&document.final_url);
    let mut header = Header {
        target,
        redirected_to,
        facts: vec![
            status_fact(document.status),
            content_type_fact(document.content_type.as_deref()),
        ],
        fresh_copy: true,
        notices: Vec::new(),
        error: None,
        wrap_toggle: false,
    };
    match &document.body {
        SourceBody::Binary { bytes } => {
            let at_least = if *bytes >= MAX_SHOWN_BYTES as u64 {
                "at least "
            } else {
                ""
            };
            header
                .facts
                .push(format!("Size: {at_least}{}", format_size_exact(*bytes)));
            header.notices.push(format!(
                "This is binary content ({}, {at_least}{}), so its source is not shown.",
                document
                    .content_type
                    .as_deref()
                    .map_or("type not sent", str::trim),
                format_size(*bytes),
            ));
            push_page_start(&mut out, target, false);
            push_header(&mut out, &header);
        }
        SourceBody::Text {
            text,
            bytes,
            truncated,
            charset,
        } => {
            let body = TextBody {
                text,
                bytes: *bytes,
                truncated: *truncated,
                charset,
            };
            build_text_page(&mut out, document, target, header, &body, colour);
        }
    }
    push_page_end(&mut out);
    out
}

/// The parts of `SourceBody::Text`.
struct TextBody<'a> {
    text: &'a str,
    bytes: u64,
    truncated: bool,
    charset: &'a str,
}

fn build_text_page(
    out: &mut String,
    document: &SourceDocument,
    target: &Url,
    mut header: Header<'_>,
    body: &TextBody<'_>,
    colour: bool,
) {
    // A byte order mark is not part of the text.
    let text = body.text.strip_prefix('\u{feff}').unwrap_or(body.text);
    let total_lines = count_lines(text);
    let shown_lines = total_lines.min(MAX_SHOWN_LINES);

    let outcome = if colour {
        highlight::highlight(document.kind, text)
    } else if has_grammar(document.kind) {
        Outcome::Failed
    } else {
        Outcome::NoGrammar
    };
    let (spans, links, colour_notice) = match outcome {
        // Links resolve against the address the document came from (after redirects), as a
        // browser resolves them; `<base>` is ignored.
        Outcome::Done(Highlighted { spans, links }) => {
            let links = ViewSourceUrl::for_page(&document.final_url)
                .map(|base| make_links(text, &base, &links))
                .unwrap_or_default();
            (spans, links, None)
        }
        Outcome::TooLarge => (
            Vec::new(),
            Vec::new(),
            Some(format!(
                "Colours and links are not shown because the source is larger than {}.",
                format_size(MAX_HIGHLIGHT_BYTES as u64)
            )),
        ),
        Outcome::Failed => (
            Vec::new(),
            Vec::new(),
            Some("Colours and links are not shown because highlighting did not finish.".to_owned()),
        ),
        Outcome::NoGrammar => (Vec::new(), Vec::new(), None),
    };

    header.facts.push(if body.truncated {
        format!(
            "Size: {} shown; the response is longer",
            format_size_exact(body.bytes)
        )
    } else {
        format!("Size: {}", format_size_exact(body.bytes))
    });
    header.facts.push(format!("Charset: {}", body.charset));
    header.facts.push(format!(
        "{} {}",
        with_commas(total_lines as u64),
        if total_lines == 1 { "line" } else { "lines" }
    ));
    if body.truncated {
        header.notices.push(format!(
            "This page is larger than {size}, so only the first {size} is shown.",
            size = format_size(MAX_SHOWN_BYTES as u64),
        ));
    }
    let cut_notice = (total_lines > shown_lines).then(|| {
        format!(
            "Only the first {} of {} lines are shown.",
            with_commas(shown_lines as u64),
            with_commas(total_lines as u64)
        )
    });
    header.notices.extend(cut_notice.clone());
    header.notices.extend(colour_notice);
    if total_lines == 0 {
        header
            .notices
            .push("The response body is empty.".to_owned());
    }
    header.wrap_toggle = shown_lines > 0;

    push_page_start(out, target, header.wrap_toggle);
    push_header(out, &header);
    if shown_lines > 0 {
        let digits = digits_of(shown_lines).min(MAX_GUTTER_DIGITS);
        let _ = write!(out, "<main>\n<div class=\"src d{digits}\">");
        push_lines(out, text, &spans, &links, MAX_SHOWN_LINES);
        out.push_str("</div>\n");
        if let Some(notice) = cut_notice {
            out.push_str("<p class=\"notice\">");
            escape_into(out, &notice);
            out.push_str("</p>\n");
        }
        out.push_str("</main>\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_ADDRESS_BYTES;

    fn escaped(text: &str) -> String {
        let mut out = String::new();
        escape_into(&mut out, text);
        out
    }

    #[test]
    fn escaping_covers_markup_quotes_and_control_characters() {
        assert_eq!(escaped("a&b<c>d\"e'f"), "a&amp;b&lt;c&gt;d&quot;e&#39;f");
        assert_eq!(escaped("tab\there"), "tab\there");
        assert_eq!(escaped("nul\u{0}x"), "nul\u{2400}x");
        assert_eq!(
            escaped("\u{1}\u{8}\u{b}\u{c}\u{1f}\u{7f}"),
            "\u{2401}\u{2408}\u{240b}\u{240c}\u{241f}\u{2421}"
        );
        assert_eq!(escaped("line\nbreak\r"), "line\u{240a}break\u{240d}");
        // Characters that are text: the line separator, the BOM, bidi controls, emoji, accents.
        let text = "\u{2028}\u{feff}\u{202e}é☃😀";
        assert_eq!(escaped(text), text);
        assert_eq!(escaped(""), "");
        assert_eq!(escaped("&&"), "&amp;&amp;");
    }

    #[test]
    fn decoding_entities_handles_what_attribute_values_use() {
        assert_eq!(decode_entities("a=1&amp;b=2"), "a=1&b=2");
        assert_eq!(decode_entities("&lt;&gt;&quot;&apos;"), "<>\"'");
        assert_eq!(decode_entities("&#47;x&#x2F;y&#X2f;"), "/x/y/");
        assert_eq!(
            decode_entities("&unknown; &amp &#0; &#xD800; &#; &"),
            "&unknown; &amp &#0; &#xD800; &#; &"
        );
        assert_eq!(decode_entities("é&amp;é"), "é&é");
        assert!(matches!(decode_entities("plain"), Cow::Borrowed(_)));
        // A multi-byte character right after the ampersand must not break slicing.
        assert_eq!(decode_entities("&ééééééé;"), "&ééééééé;");
    }

    #[test]
    fn sizes_and_numbers_are_formatted_for_people() {
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(1000), "1,000");
        assert_eq!(with_commas(1_234_567), "1,234,567");
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(12_596), "12.3 KiB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MiB");
        assert_eq!(format_size_exact(12_596), "12.3 KiB (12,596 bytes)");
        assert_eq!(format_size_exact(7), "7 bytes");
        assert_eq!(digits_of(0), 1);
        assert_eq!(digits_of(9), 1);
        assert_eq!(digits_of(10), 2);
        assert_eq!(digits_of(50_000), 5);
    }

    #[test]
    fn lines_split_on_lf_crlf_and_lone_cr_and_a_final_break_adds_no_line() {
        let lines = |text: &str| -> Vec<String> {
            let mut out = Vec::new();
            let mut pos = 0;
            while let Some((start, end, next)) = next_line(text, pos) {
                out.push(text[start..end].to_owned());
                pos = next;
            }
            out
        };
        assert_eq!(lines(""), Vec::<String>::new());
        assert_eq!(lines("a"), ["a"]);
        assert_eq!(lines("a\n"), ["a"]);
        assert_eq!(lines("\n"), [""]);
        assert_eq!(lines("a\nb"), ["a", "b"]);
        assert_eq!(lines("a\r\nb\r\n"), ["a", "b"]);
        assert_eq!(lines("a\rb\r"), ["a", "b"]);
        assert_eq!(lines("a\r\rb"), ["a", "", "b"]);
        assert_eq!(lines("a\n\r\nb"), ["a", "", "b"]);
        assert_eq!(lines("a\u{2028}b"), ["a\u{2028}b"]);
        assert_eq!(count_lines("a\nb\r\nc\rd"), 4);
    }

    #[test]
    fn titles_are_cut_to_sixty_characters() {
        let short = Url::parse("https://example.com/a/b?q=1#f").unwrap();
        assert_eq!(page_title(&short), "Source of example.com/a/b");
        let long = Url::parse(&format!("https://example.com/{}", "ü".repeat(100))).unwrap();
        let title = page_title(&long);
        assert_eq!(title.chars().count(), TITLE_LIMIT);
        assert!(title.ends_with('\u{2026}') && title.starts_with("Source of example.com/"));
        let fit = "a".repeat(TITLE_LIMIT - "Source of h.test/".len());
        let exact = Url::parse(&format!("https://h.test/{fit}")).unwrap();
        assert_eq!(page_title(&exact).chars().count(), TITLE_LIMIT);
        assert!(!page_title(&exact).ends_with('\u{2026}'));
    }

    #[test]
    fn links_resolve_against_the_page_and_only_http_and_https_count() {
        let base = ViewSourceUrl::for_page(
            &Url::parse("https://example.com/dir/page.html?x=1#top").unwrap(),
        )
        .unwrap();
        let link = |raw: &str| resolve_link(&base, raw);
        assert_eq!(
            link("/a b").as_deref(),
            Some("view-source:https://example.com/a%20b")
        );
        assert_eq!(
            link("//x.test/").as_deref(),
            Some("view-source:https://x.test/")
        );
        assert_eq!(
            link("../up/../z.css").as_deref(),
            Some("view-source:https://example.com/z.css")
        );
        assert_eq!(
            link("sub/p.js?q=a&amp;b=2").as_deref(),
            Some("view-source:https://example.com/dir/sub/p.js?q=a&b=2")
        );
        assert_eq!(
            link("  /spaced  ").as_deref(),
            Some("view-source:https://example.com/spaced")
        );
        assert_eq!(
            link("http://other.test:8080/p#frag").as_deref(),
            Some("view-source:http://other.test:8080/p")
        );
        assert_eq!(
            link("https://user:pw@example.com/p").as_deref(),
            Some("view-source:https://example.com/p")
        );
        assert_eq!(
            link("?only=query").as_deref(),
            Some("view-source:https://example.com/dir/page.html?only=query")
        );
        for refused in [
            "javascript:alert(1)",
            "JAVASCRIPT:1",
            "data:text/html,x",
            "mailto:a@b.test",
            "tel:1",
            "ftp://x.test/",
            "file:///etc/hosts",
            "blob:https://x.test/u",
            "about:blank",
            "view-source:https://x.test/",
            "#frag",
            "#",
            "",
            "   ",
            "http://",
            "https://[::1",
        ] {
            assert_eq!(link(refused), None, "{refused:?}");
        }
        // An address over the limit is not a link.
        let long = format!("/{}", "a".repeat(MAX_ADDRESS_BYTES));
        assert_eq!(link(&long), None);
        // The base's own credentials and fragment never leak into a link.
        let with_credentials =
            ViewSourceUrl::for_page(&Url::parse("https://u:p@example.com/x#f").unwrap()).unwrap();
        assert_eq!(
            resolve_link(&with_credentials, "y").as_deref(),
            Some("view-source:https://example.com/y")
        );
    }

    #[test]
    fn status_and_size_facts_read_well() {
        assert_eq!(status_fact(200), "Status: 200 OK");
        assert_eq!(status_fact(404), "Status: 404 Not Found");
        assert_eq!(status_fact(299), "Status: 299");
        assert_eq!(content_type_fact(None), "Content type: not sent");
        assert_eq!(content_type_fact(Some("  ")), "Content type: not sent");
        assert_eq!(
            content_type_fact(Some("text/html; charset=utf-8")),
            "Content type: text/html; charset=utf-8"
        );
    }

    fn text_document(kind: SourceKind, text: &str) -> SourceDocument {
        SourceDocument {
            final_url: Url::parse("https://example.test/dir/index.html").unwrap(),
            status: 200,
            content_type: Some("text/html".to_owned()),
            kind,
            body: SourceBody::Text {
                text: text.to_owned(),
                bytes: text.len() as u64,
                truncated: false,
                charset: "UTF-8",
            },
        }
    }

    #[test]
    fn a_panic_while_colouring_still_gives_a_plain_page() {
        let document = text_document(
            SourceKind::Html,
            &format!("<p>{}</p>", highlight::PANIC_MARKER),
        );
        let page = build_page(&document, &document.final_url.clone(), true);
        assert!(
            page.contains("Colours and links are not shown because highlighting did not finish.")
        );
        assert!(page.contains(highlight::PANIC_MARKER));
        assert!(!page.contains("<span class=\"t\">"));
        // The first line is there, with its number.
        assert!(page.contains("<div class=\"l\" data-n=\"1\"><span>&lt;p&gt;"));
    }

    #[test]
    fn the_plain_second_try_marks_a_grammar_kind_but_not_a_plain_one() {
        let target = Url::parse("https://example.test/dir/index.html").unwrap();
        let html = text_document(SourceKind::Html, "<p>x</p>");
        assert!(build_page(&html, &target, false).contains("did not finish"));
        let plain = text_document(SourceKind::Plain, "{}");
        assert!(!build_page(&plain, &target, false).contains("Colours"));
    }

    #[test]
    fn spans_and_links_that_cross_lines_are_closed_and_reopened_on_each_line() {
        let text = "ab\ncd\nef";
        let spans = [Span {
            start: 1,
            end: 7,
            class: 1,
        }];
        let links = [LinkRange {
            start: 0,
            end: 8,
            href: "view-source:https://x.test/".to_owned(),
        }];
        let mut out = String::new();
        let shown = push_lines(&mut out, text, &spans, &links, 10);
        assert_eq!(shown, 3);
        let a = "<a href=\"view-source:https://x.test/\">";
        let expected = format!(
            "<div class=\"l\" data-n=\"1\"><span>{a}a<span class=\"k\">b</span></a></span></div>\
             <div class=\"l\" data-n=\"2\"><span>{a}<span class=\"k\">cd</span></a></span></div>\
             <div class=\"l\" data-n=\"3\"><span>{a}<span class=\"k\">e</span>f</a></span></div>"
        );
        assert_eq!(out, expected);
    }
}
