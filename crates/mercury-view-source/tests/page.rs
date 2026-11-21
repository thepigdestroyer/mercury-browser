// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The source page: what `render_page`, `render_error_page` and `page_headers` produce for hostile
//! and ordinary sources. Every generated page is parsed back with the `tree-sitter-html` grammar:
//! no `ERROR` node, and no element outside the fixed set the generator writes.
//!
//! The lines of a page are read back (tags dropped, entities decoded) and compared with the
//! source, so "appears only as text" is checked on the markup the browser will get.

use std::time::{Duration, Instant};

use mercury_view_source::{
    FetchError, MAX_ADDRESS_BYTES, MAX_HIGHLIGHT_BYTES, MAX_LINKS, MAX_SHOWN_BYTES,
    MAX_SHOWN_LINES, SourceBody, SourceDocument, SourceKind, ViewSourceUrl, page_headers,
    render_error_page, render_page,
};
use tree_sitter::{Node, Parser, Tree};
use url::Url;

/// The tags the generator may write, and nothing else (the brief).
const FIXED_TAGS: &[&str] = &[
    "html", "head", "meta", "title", "style", "body", "header", "h1", "p", "a", "label", "input",
    "main", "div", "span",
];
/// The attributes the generator may write: no event handler, no `style`, no `src`.
const FIXED_ATTRIBUTES: &[&str] = &[
    "lang", "charset", "name", "content", "type", "id", "checked", "for", "class", "data-n", "href",
];
const PAGE_ADDRESS: &str = "https://example.test/dir/page.html";
const FRESH_COPY: &str =
    "This is a fresh copy fetched from the server, so it can differ from the page you see";

fn request(address: &str) -> ViewSourceUrl {
    ViewSourceUrl::parse(&format!("view-source:{address}")).expect("a valid view-source address")
}

fn document(kind: SourceKind, text: &str) -> SourceDocument {
    SourceDocument {
        final_url: Url::parse(PAGE_ADDRESS).expect("url"),
        status: 200,
        content_type: Some("text/html; charset=utf-8".to_owned()),
        kind,
        body: SourceBody::Text {
            text: text.to_owned(),
            bytes: text.len() as u64,
            truncated: false,
            charset: "UTF-8",
        },
    }
}

fn render_document(document: &SourceDocument) -> String {
    render_page(document, &request(PAGE_ADDRESS)).html
}

fn render(kind: SourceKind, text: &str) -> String {
    render_document(&document(kind, text))
}

fn parse(html: &str) -> Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_html::LANGUAGE.into())
        .expect("html grammar");
    parser.parse(html, None).expect("a parse")
}

/// Every node of the tree, depth first, without recursion.
fn nodes(tree: &Tree) -> Vec<Node<'_>> {
    let mut found = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        found.push(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    found
}

/// The page parses with no error and writes only the fixed set of tags; returns the tree.
fn assert_well_formed(html: &str) -> Tree {
    let tree = parse(html);
    let mut problems = Vec::new();
    for node in nodes(&tree) {
        if node.is_error() || node.is_missing() {
            problems.push(format!("{} at byte {}", node.kind(), node.start_byte()));
            if problems.len() > 5 {
                break;
            }
        }
        match node.kind() {
            "tag_name" | "erroneous_end_tag_name" => {
                let name = html[node.byte_range()].to_ascii_lowercase();
                if !FIXED_TAGS.contains(&name.as_str()) {
                    problems.push(format!("element <{name}> is not in the fixed set"));
                }
            }
            "attribute_name" => {
                let name = html[node.byte_range()].to_ascii_lowercase();
                if !FIXED_ATTRIBUTES.contains(&name.as_str()) {
                    problems.push(format!("attribute {name} is not in the fixed set"));
                }
            }
            "script_element" => problems.push("a script element".to_owned()),
            _ => {}
        }
    }
    assert!(
        problems.is_empty(),
        "the page is not well formed: {problems:?}"
    );
    assert!(!tree.root_node().has_error());
    tree
}

fn element_names(html: &str) -> std::collections::BTreeSet<String> {
    let tree = parse(html);
    nodes(&tree)
        .into_iter()
        .filter(|node| node.kind() == "tag_name")
        .map(|node| html[node.byte_range()].to_ascii_lowercase())
        .collect()
}

fn strip_tags(markup: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in markup.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

fn decode(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The text of every line of the page, as a reader sees it.
fn lines_of(html: &str) -> Vec<String> {
    let marker = "<div class=\"l\" data-n=\"";
    html.split(marker)
        .skip(1)
        .map(|chunk| {
            let after = chunk.split_once("\"><span>").expect("line markup").1;
            let body = after.split_once("</span></div>").expect("line end").0;
            decode(&strip_tags(body))
        })
        .collect()
}

/// What the lines of `text` should read as: split on LF, CRLF and CR; control characters other
/// than the tab shown as their control picture.
fn expected_lines(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut pending = false;
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            c if (c as u32) < 0x20 && c != '\t' => {
                current.push(char::from_u32(0x2400 + c as u32).expect("control picture"));
                pending = true;
            }
            '\u{7f}' => {
                current.push('\u{2421}');
                pending = true;
            }
            c => {
                current.push(c);
                pending = true;
            }
        }
    }
    if pending {
        lines.push(current);
    }
    lines
}

/// The addresses of the `<a href>` elements inside the lines (not the header).
fn source_links(html: &str) -> Vec<String> {
    let body = html.split_once("<main>").map_or("", |(_, rest)| rest);
    body.split("<a href=\"")
        .skip(1)
        .map(|rest| decode(rest.split_once('"').expect("end of href").0))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Cases.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_table_of_sources_renders_well_formed_pages_whose_lines_read_back_as_the_source() {
    let inline = "<!doctype html>\n<html>\n<head><title>t</title>\n<style>a{color:red}</style>\n\
                  <script>var x = 1;\nif (x < 2 && x > 0) { alert(\"hi\"); }</script></head>\n\
                  <body><a href=\"/next\">next</a></body></html>\n";
    let minified = format!("var a=1;{}", "a=a+1;".repeat(60_000)); // about 360 KiB on one line
    let cases: Vec<(&str, SourceKind, String)> = vec![
        (
            "html with inline script and style",
            SourceKind::Html,
            inline.to_owned(),
        ),
        (
            "plain text",
            SourceKind::Plain,
            "just\nsome words\n".to_owned(),
        ),
        (
            "json",
            SourceKind::Plain,
            "{\"a\": [1, 2, {\"b\": null}], \"c\": \"<x>\"}".to_owned(),
        ),
        (
            "javascript",
            SourceKind::JavaScript,
            "const f = (a, b) => a < b ? `x${a}` : /re+/g;\n".to_owned(),
        ),
        (
            "css",
            SourceKind::Css,
            "@media (min-width: 1px) { a > b { color: #fff } }\n".to_owned(),
        ),
        (
            "svg",
            SourceKind::Xml,
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M0 0\"/></svg>".to_owned(),
        ),
        (
            "one 360 KiB minified line",
            SourceKind::JavaScript,
            minified,
        ),
        ("empty", SourceKind::Html, String::new()),
        ("only a newline", SourceKind::Plain, "\n".to_owned()),
        ("only blank lines", SourceKind::Html, "\n\n\n".to_owned()),
        ("crlf", SourceKind::Plain, "a\r\nb\r\n\r\nc".to_owned()),
        ("lone cr", SourceKind::Plain, "a\rb\r\rc\r".to_owned()),
        (
            "mixed breaks",
            SourceKind::Html,
            "<p>\r\n<b>\r<i>\n</i></b></p>".to_owned(),
        ),
        (
            "tabs",
            SourceKind::Plain,
            "\tindented\t\tmore\n\t\t".to_owned(),
        ),
        (
            "nul and other controls",
            SourceKind::Plain,
            "a\u{0}b\u{1}c\u{7f}d\u{1b}[0m\n".to_owned(),
        ),
        (
            "bom",
            SourceKind::Html,
            "\u{feff}<!doctype html><p>bom</p>".to_owned(),
        ),
        (
            "unicode",
            SourceKind::Html,
            "<p title=\"héllo ☃ 😀\">日本語\u{2028}\u{202e}</p>".to_owned(),
        ),
        (
            "unterminated markup",
            SourceKind::Html,
            "<div class=\"x><p>unterminated <!-- comment".to_owned(),
        ),
        (
            "multi-line comment",
            SourceKind::JavaScript,
            "/* one\ntwo\nthree */ let x;".to_owned(),
        ),
        (
            "multi-line template",
            SourceKind::JavaScript,
            "let s = `a\nb${1}\nc`;".to_owned(),
        ),
    ];
    for (name, kind, text) in cases {
        let html = render(kind, &text);
        assert_well_formed(&html);
        assert_eq!(
            lines_of(&html),
            expected_lines(&text),
            "{name}: lines read back"
        );
        // The shared furniture of every source page.
        assert!(html.contains(FRESH_COPY), "{name}: fresh copy sentence");
        assert!(!html.contains("<script"), "{name}: a script tag");
        assert!(!html.contains('\u{0}'), "{name}: a raw NUL");
        assert_eq!(
            html.matches("<style>").count(),
            1,
            "{name}: one style element"
        );
    }
}

#[test]
fn a_hostile_source_cannot_add_markup_in_any_kind_of_page() {
    let hostile = "</pre></div><script>alert(1)</script><img src=x onerror=1>";
    for kind in [
        SourceKind::Html,
        SourceKind::Xml,
        SourceKind::JavaScript,
        SourceKind::Css,
        SourceKind::Plain,
    ] {
        let html = render(kind, hostile);
        assert_well_formed(&html);
        assert_eq!(lines_of(&html), [hostile], "{kind:?}");
        if kind == SourceKind::Plain {
            // (With a grammar, the tokens split the escaped text with spans; the lines read back above.)
            assert!(html.contains("&lt;/pre&gt;&lt;/div&gt;&lt;script&gt;"));
        }
        for forbidden in ["<img", "<script", "</pre", "onerror=1>"] {
            assert!(!html.contains(forbidden), "{kind:?}: {forbidden}");
        }
        assert!(
            element_names(&html).is_subset(&FIXED_TAGS.iter().map(|t| (*t).to_owned()).collect()),
            "{kind:?}"
        );
    }
}

#[test]
fn each_special_character_appears_only_as_text_in_text_and_in_links() {
    let specials = ["&", "<", ">", "\"", "'", "\u{2028}", "\u{0}"];
    for special in specials {
        for kind in [
            SourceKind::Html,
            SourceKind::Plain,
            SourceKind::JavaScript,
            SourceKind::Css,
        ] {
            let text = format!("<a href=\"/p{special}q\">x{special}y</a> {special}{special}");
            let html = render(kind, &text);
            assert_well_formed(&html);
            assert_eq!(
                lines_of(&html),
                expected_lines(&text),
                "{special:?} in {kind:?}"
            );
        }
        // In an attribute (the link) and in the title and the header of the page.
        let attribute = format!("<a href='/x{special}y'>z</a>");
        let html = render(SourceKind::Html, &attribute);
        assert_well_formed(&html);
        for link in source_links(&html) {
            assert!(
                link.starts_with("view-source:https://example.test/"),
                "{link}"
            );
            assert!(
                !link.contains(['<', '>', '"', '\'', '\u{0}']),
                "{special:?}: {link}"
            );
        }
    }
    // An address with the characters in it, in the page's own header and title.
    let weird = request("https://example.test/a%22b'c<d>e&f?g=\"h'#frag");
    let html = render_page(&document(SourceKind::Plain, "x"), &weird).html;
    assert_well_formed(&html);
}

#[test]
fn text_from_the_server_is_escaped_in_the_header() {
    let mut hostile = document(SourceKind::Plain, "x");
    hostile.content_type = Some("text/plain\"><script>alert(1)</script><b onload=x>".to_owned());
    let html = render_document(&hostile);
    assert_well_formed(&html);
    assert!(html.contains("text/plain&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;"));
    let mut binary = document(SourceKind::Binary, "");
    binary.content_type = Some("image/png\"><img src=x onerror=1>".to_owned());
    binary.body = SourceBody::Binary { bytes: 10 };
    let html = render_document(&binary);
    assert_well_formed(&html);
    assert!(!html.contains("<img"));
    // A redirect to an address with markup characters in it.
    let mut redirected = document(SourceKind::Plain, "x");
    redirected.final_url = Url::parse("https://other.test/a\"b<c>d'e?f=\"<script>").expect("url");
    let html = render_document(&redirected);
    assert_well_formed(&html);
    assert!(html.contains("Redirected to"));
    assert!(!html.contains("<script>"));
}

#[test]
fn links_follow_the_table() {
    let source = "\
<a href=\"/a b\">1</a>
<a href=\"//x.test/\">2</a>
<a href=\"javascript:alert(1)\">3</a>
<a href=\"data:text/html,x\">4</a>
<a href=\"mailto:a@b.test\">5</a>
<a href=\"#frag\">6</a>
<a href=\"../up/../z.css?a=1&amp;b=2#f\">7</a>
<img src=\"http://other.test:8080/i.png\">
<a href=\"\">8</a><a href>9</a>
<a href=sub/page.html>10</a>
<script src=\"/s.js\"></script>
<a href=\"https://u:p@cred.test/p\">11</a>
<a href=\"ftp://x.test/\">12</a>
<a href=\"view-source:https://x.test/\">13</a>
";
    let html = render(SourceKind::Html, source);
    assert_well_formed(&html);
    assert_eq!(
        source_links(&html),
        [
            "view-source:https://example.test/a%20b",
            "view-source:https://x.test/",
            "view-source:https://example.test/z.css?a=1&b=2",
            "view-source:http://other.test:8080/i.png",
            "view-source:https://example.test/dir/sub/page.html",
            "view-source:https://example.test/s.js",
            "view-source:https://cred.test/p",
        ]
    );
    // The text of the links is still the source, unchanged.
    assert_eq!(lines_of(&html), expected_lines(source));
    // `<base>` is ignored: its own `href` is a link like any other, and the next link still
    // resolves against the page.
    let with_base = "<base href=\"https://evil.test/\"><a href=\"/x\">x</a>";
    let html = render(SourceKind::Html, with_base);
    assert_well_formed(&html);
    assert_eq!(
        source_links(&html),
        [
            "view-source:https://evil.test/",
            "view-source:https://example.test/x"
        ]
    );
}

#[test]
fn links_resolve_against_the_final_address_after_a_redirect() {
    let mut moved = document(
        SourceKind::Html,
        "<a href=\"next\">n</a><img src=\"/i.png\">",
    );
    moved.final_url = Url::parse("https://cdn.test/deep/dir/index.html").expect("url");
    let html = render_page(&moved, &request("http://example.test/old")).html;
    assert_well_formed(&html);
    assert_eq!(
        source_links(&html),
        [
            "view-source:https://cdn.test/deep/dir/next",
            "view-source:https://cdn.test/i.png"
        ]
    );
    // The header names both addresses and links the second.
    assert!(
        html.contains(
            "Redirected to <a href=\"view-source:https://cdn.test/deep/dir/index.html\">"
        )
    );
}

#[test]
fn six_thousand_links_are_cut_at_the_cap() {
    let source: String = (0..6000)
        .map(|n| format!("<a href=\"/p{n}\">{n}</a>\n"))
        .collect();
    let html = render(SourceKind::Html, &source);
    assert_well_formed(&html);
    let links = source_links(&html);
    assert_eq!(links.len(), MAX_LINKS);
    assert_eq!(links[0], "view-source:https://example.test/p0");
    assert_eq!(
        links[MAX_LINKS - 1],
        format!("view-source:https://example.test/p{}", MAX_LINKS - 1)
    );
    assert_eq!(lines_of(&html).len(), 6000, "the lines are all there");
}

#[test]
fn links_with_long_addresses_stay_within_the_size_bound() {
    // 5,000 links, each resolved against an address near the 8 KiB limit.
    let mut long = document(SourceKind::Html, &"<a href=\"x\">l</a>\n".repeat(5000));
    long.final_url = Url::parse(&format!(
        "https://example.test/{}/",
        "d".repeat(MAX_ADDRESS_BYTES - 200)
    ))
    .expect("url");
    let html = render_document(&long);
    assert_well_formed(&html);
    let links = source_links(&html);
    assert!(
        !links.is_empty() && links.len() < 5000,
        "{} links",
        links.len()
    );
    assert!(html.len() < 24 * 1024 * 1024, "{} bytes", html.len());
}

#[test]
fn long_sources_are_cut_at_the_line_limit_with_a_notice() {
    let text: String = (1..=60_000).map(|n| format!("line {n}\n")).collect();
    let html = render(SourceKind::Plain, &text);
    assert_well_formed(&html);
    let lines = lines_of(&html);
    assert_eq!(lines.len(), MAX_SHOWN_LINES);
    assert_eq!(
        lines[MAX_SHOWN_LINES - 1],
        format!("line {MAX_SHOWN_LINES}")
    );
    assert!(html.contains("<div class=\"l\" data-n=\"50000\">"));
    assert!(!html.contains("data-n=\"50001\""));
    assert!(html.contains("Only the first 50,000 of 60,000 lines are shown."));
    assert!(html.contains("<div class=\"src d5\">"));
    // Exactly at the limit: no notice.
    let exact: String = (1..=MAX_SHOWN_LINES).map(|n| format!("{n}\n")).collect();
    let html = render(SourceKind::Plain, &exact);
    assert!(!html.contains("lines are shown"));
    assert_eq!(lines_of(&html).len(), MAX_SHOWN_LINES);
}

#[test]
fn line_numbers_are_data_attributes_and_the_gutter_class_follows_the_digits() {
    let html = render(SourceKind::Plain, "a\nb\nc");
    assert!(html.contains("<div class=\"l\" data-n=\"1\"><span>a</span></div>"));
    assert!(html.contains("<div class=\"l\" data-n=\"3\"><span>c</span></div>"));
    assert!(html.contains("<div class=\"src d1\">"));
    let html = render(SourceKind::Plain, &"x\n".repeat(120));
    assert!(html.contains("<div class=\"src d3\">"));
    // An empty line is still a line, with an empty span for the grid.
    let html = render(SourceKind::Plain, "a\n\nb");
    assert!(html.contains("<div class=\"l\" data-n=\"2\"><span></span></div>"));
    // No counters anywhere.
    assert!(!html.contains("counter"));
}

#[test]
fn the_wrap_toggle_is_a_checked_checkbox_before_the_header_and_the_lines() {
    let html = render(SourceKind::Html, "<p>x</p>");
    let input = html
        .find("<input type=\"checkbox\" id=\"wrap\" checked>")
        .expect("checkbox");
    let header = html.find("<header>").expect("header");
    let main = html.find("<main>").expect("main");
    assert!(
        input < header && header < main,
        "the checkbox precedes what it styles"
    );
    assert!(html.contains("<label for=\"wrap\">Wrap long lines</label>"));
    assert!(html.contains("#wrap:not(:checked)~main .l{white-space:pre;"));
    // Pages without lines have no toggle.
    let empty = render(SourceKind::Plain, "");
    assert!(!empty.contains("id=\"wrap\"") && !empty.contains("<label"));
}

#[test]
fn the_header_names_the_address_status_type_size_charset_and_the_fresh_copy_sentence() {
    let mut doc = document(SourceKind::Html, "<p>hello</p>\n");
    doc.final_url = Url::parse("https://example.test/dir/page.html?q=1").expect("url");
    doc.status = 404;
    doc.content_type = Some("text/html; charset=iso-8859-1".to_owned());
    if let SourceBody::Text { charset, .. } = &mut doc.body {
        *charset = "windows-1252";
    }
    let html = render_page(&doc, &request("https://example.test/dir/page.html?q=1#f")).html;
    assert_well_formed(&html);
    let header = html
        .split_once("<header>")
        .and_then(|(_, rest)| rest.split_once("</header>"))
        .expect("header")
        .0;
    assert!(
        header.contains("<p class=\"addr\">https://example.test/dir/page.html?q=1</p>"),
        "{header}"
    );
    assert!(header.contains("Status: 404 Not Found"));
    assert!(header.contains("Content type: text/html; charset=iso-8859-1"));
    assert!(header.contains("Size: 13 bytes"));
    assert!(header.contains("Charset: windows-1252"));
    assert!(header.contains("1 line"));
    assert!(header.contains(&format!("{FRESH_COPY}.")));
    // A page that was not redirected says nothing about a redirect.
    assert!(!header.contains("Redirected"));
    // The sentence is on every source page, including binary and empty ones.
    let mut binary = document(SourceKind::Binary, "");
    binary.body = SourceBody::Binary { bytes: 4096 };
    assert!(render_document(&binary).contains(FRESH_COPY));
    assert!(render(SourceKind::Plain, "").contains(FRESH_COPY));
}

#[test]
fn a_binary_page_says_the_type_and_size_and_shows_no_body() {
    let mut png = document(SourceKind::Binary, "");
    png.content_type = Some("image/png".to_owned());
    png.body = SourceBody::Binary { bytes: 12_596 };
    let html = render_document(&png);
    assert_well_formed(&html);
    assert!(
        html.contains("This is binary content (image/png, 12.3 KiB), so its source is not shown.")
    );
    assert!(html.contains("Size: 12.3 KiB (12,596 bytes)"));
    assert!(!html.contains("<main>") && !html.contains("data-n=") && !html.contains("<label"));
    // At the cap the size is a lower bound; with no type the page says so.
    let mut unknown = document(SourceKind::Binary, "");
    unknown.content_type = None;
    unknown.body = SourceBody::Binary {
        bytes: MAX_SHOWN_BYTES as u64 + 1,
    };
    let html = render_document(&unknown);
    assert_well_formed(&html);
    assert!(html.contains("type not sent, at least 2.0 MiB"));
    assert!(html.contains("Content type: not sent"));
}

#[test]
fn a_truncated_source_has_a_notice_and_an_empty_one_says_so() {
    let mut doc = document(SourceKind::Plain, "partial");
    if let SourceBody::Text {
        truncated, bytes, ..
    } = &mut doc.body
    {
        *truncated = true;
        *bytes = MAX_SHOWN_BYTES as u64;
    }
    let html = render_document(&doc);
    assert_well_formed(&html);
    assert!(html.contains("This page is larger than 2.0 MiB, so only the first 2.0 MiB is shown."));
    assert!(html.contains("Size: 2.0 MiB (2,097,152 bytes) shown; the response is longer"));
    let empty = render(SourceKind::Html, "");
    assert_well_formed(&empty);
    assert!(empty.contains("The response body is empty."));
    assert!(!empty.contains("<main>"));
}

#[test]
fn the_title_is_source_of_host_and_path_cut_to_sixty_characters() {
    let title_of = |html: &str| -> String {
        let after = html.split_once("<title>").expect("title").1;
        decode(after.split_once("</title>").expect("end").0)
    };
    let html = render_page(
        &document(SourceKind::Plain, "x"),
        &request("https://example.com/a/b?q=1#f"),
    )
    .html;
    assert_eq!(title_of(&html), "Source of example.com/a/b");
    let long = request(&format!("https://example.com/{}", "segment/".repeat(30)));
    let html = render_page(&document(SourceKind::Plain, "x"), &long).html;
    let title = title_of(&html);
    assert_eq!(title.chars().count(), 60, "{title}");
    assert!(title.ends_with('\u{2026}'));
    // Also for the error page.
    let html = render_error_page(&long, &FetchError::Timeout).html;
    assert_eq!(title_of(&html).chars().count(), 60);
}

#[test]
fn every_fetch_error_has_a_page_with_one_fixed_sentence() {
    let errors = [
        FetchError::Unreachable,
        FetchError::Timeout,
        FetchError::Tls,
        FetchError::TooManyRedirects,
        FetchError::BadRedirect,
        FetchError::Proxy,
        FetchError::Internal,
    ];
    let req = request("https://example.test/dir/page.html?secret=1");
    for error in errors {
        let html = render_error_page(&req, &error).html;
        assert_well_formed(&html);
        assert!(
            html.contains(&format!("<p class=\"err\">{}</p>", error.summary())),
            "{error:?}"
        );
        assert!(html.contains("<p class=\"addr\">https://example.test/dir/page.html?secret=1</p>"));
        assert!(
            !html.contains("<main>") && !html.contains("data-n=") && !html.contains(FRESH_COPY)
        );
        assert!(html.contains("<title>Source of example.test/dir/page.html</title>"));
        assert_eq!(html.matches("<p class=\"err\">").count(), 1);
    }
}

#[test]
fn the_headers_are_exactly_the_ones_the_brief_names() {
    let headers = page_headers();
    assert_eq!(
        headers,
        [
            ("content-type", "text/html; charset=utf-8"),
            (
                "content-security-policy",
                "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
            ),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
        ]
    );
    // Header names are lower case and values have no line breaks (they go into a response).
    for (name, value) in headers {
        assert_eq!(name, name.to_ascii_lowercase());
        assert!(!value.contains(['\r', '\n']));
    }
}

#[test]
fn nothing_the_policy_forbids_is_in_the_page() {
    // default-src 'none': no script, image, font, frame, form or base in any page we generate.
    let html = render(
        SourceKind::Html,
        "<script src=a.js></script><img src=b.png><link rel=stylesheet href=c.css><form action=/x><iframe src=/f>",
    );
    assert_well_formed(&html);
    for banned in [
        "<script",
        "<img",
        "<link",
        "<form",
        "<iframe",
        "<base",
        "<object",
        "<embed",
        "<svg",
        "<video",
        "<audio",
        "<meta http-equiv",
        "@import",
        "url(",
        "javascript:",
    ] {
        assert!(!html.contains(banned), "{banned}");
    }
    let names = element_names(&html);
    assert!(
        names.is_subset(&FIXED_TAGS.iter().map(|t| (*t).to_owned()).collect()),
        "{names:?}"
    );
}

#[test]
fn a_two_mebibyte_source_is_plain_at_once_and_the_page_stays_bounded() {
    let line = "0123456789abcdef".repeat(8); // 128 bytes
    let mut text = String::new();
    while text.len() < MAX_SHOWN_BYTES {
        text.push_str(&line);
        text.push('\n');
    }
    text.truncate(MAX_SHOWN_BYTES);
    for kind in [
        SourceKind::Html,
        SourceKind::JavaScript,
        SourceKind::Css,
        SourceKind::Plain,
    ] {
        let started = Instant::now();
        let html = render(kind, &text);
        let took = started.elapsed();
        eprintln!("2 MiB {kind:?}: {} bytes of page in {took:?}", html.len());
        assert!(
            html.len() < 24 * 1024 * 1024,
            "{kind:?}: {} bytes",
            html.len()
        );
        assert!(took < Duration::from_secs(10), "{kind:?} took {took:?}");
        if kind != SourceKind::Plain {
            assert!(
                html.contains("larger than 512.0 KiB"),
                "{kind:?}: plain, with a notice"
            );
        }
        assert!(!html.contains("<span class=\"k\">") && !html.contains("<span class=\"t\">"));
        assert!(!html.contains("<a href=\"view-source"));
        assert_eq!(
            lines_of(&html).len(),
            MAX_SHOWN_BYTES / 129 + usize::from(!MAX_SHOWN_BYTES.is_multiple_of(129))
        );
    }
}

#[test]
fn the_worst_two_mebibyte_page_for_escaping_stays_under_the_bound() {
    // Every byte becomes a six-byte entity; lines of one character make the markup heaviest too.
    let quotes = "\"".repeat(MAX_SHOWN_BYTES);
    let started = Instant::now();
    let html = render(SourceKind::Plain, &quotes);
    eprintln!(
        "2 MiB of quotes: {} bytes in {:?}",
        html.len(),
        started.elapsed()
    );
    assert!(html.len() < 24 * 1024 * 1024, "{} bytes", html.len());
    let short_lines = "\"\n".repeat(MAX_SHOWN_BYTES / 2);
    let started = Instant::now();
    let html = render(SourceKind::Plain, &short_lines);
    eprintln!(
        "2 MiB of one-character lines: {} bytes in {:?}",
        html.len(),
        started.elapsed()
    );
    assert!(html.len() < 24 * 1024 * 1024, "{} bytes", html.len());
    assert_eq!(lines_of(&html).len(), MAX_SHOWN_LINES);
}

#[test]
fn five_hundred_twelve_kib_plus_one_byte_is_plain_at_once() {
    let text = format!("<p class=\"x\">{}</p>", "a".repeat(MAX_HIGHLIGHT_BYTES));
    assert!(text.len() > MAX_HIGHLIGHT_BYTES);
    let started = Instant::now();
    let html = render(SourceKind::Html, &text);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(2), "took {took:?}");
    assert!(
        html.contains(
            "Colours and links are not shown because the source is larger than 512.0 KiB."
        )
    );
    assert!(!html.contains("<span class=\"t\">"));
    assert_well_formed(&html);
}

#[test]
fn the_generation_time_and_size_for_a_highlighted_source_are_bounded() {
    // 500 KiB of dense HTML: the heaviest highlighted page.
    let unit = "<div class=\"a\" id='b'><a href=\"/x\">t&amp;t</a><!-- c --></div>\n";
    let mut text = String::new();
    while text.len() + unit.len() < MAX_HIGHLIGHT_BYTES {
        text.push_str(unit);
    }
    let started = Instant::now();
    let html = render(SourceKind::Html, &text);
    let took = started.elapsed();
    eprintln!(
        "{} KiB of dense html: {} bytes of page in {took:?}",
        text.len() / 1024,
        html.len()
    );
    assert!(html.len() < 24 * 1024 * 1024);
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_eq!(lines_of(&html).len(), text.lines().count());
    assert!(html.contains("<span class=\"t\">div</span>"));
}
